#include "magictts.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <stdint.h>
#include <math.h>

#define SAFETENSORS_MAX_TENSORS 512

typedef enum {
    DTYPE_F32 = 0,
    DTYPE_F16 = 1,
    DTYPE_BF16 = 2,
    DTYPE_I32 = 3,
    DTYPE_I64 = 4,
    DTYPE_BOOL = 5,
    DTYPE_UNKNOWN = -1
} safetensor_dtype_t;

typedef struct {
    char name[128];
    safetensor_dtype_t dtype;
    int ndim;
    int64_t shape[8];
    size_t data_offset;
    size_t data_size;
} safetensor_t;

typedef struct {
    char *path;
    void *data;
    size_t file_size;
    size_t header_size;
    char *header_json;
    int num_tensors;
    safetensor_t tensors[SAFETENSORS_MAX_TENSORS];
} safetensors_file_t;

struct magictts_model {
    safetensors_file_t *sf;
    void *metal_context; // 指向 Metal 运行时的指针
};

// JSON 解析辅助函数
static void skip_whitespace(const char **p) {
    while (**p == ' ' || **p == '\n' || **p == '\r' || **p == '\t') (*p)++;
}

static int parse_string(const char **p, char *out, size_t max_len) {
    skip_whitespace(p);
    if (**p != '"') return -1;
    (*p)++;
    size_t i = 0;
    while (**p && **p != '"' && i < max_len - 1) {
        if (**p == '\\') {
            (*p)++;
            if (**p == 'n') out[i++] = '\n';
            else if (**p == 't') out[i++] = '\t';
            else if (**p == 'r') out[i++] = '\r';
            else if (**p == '"') out[i++] = '"';
            else if (**p == '\\') out[i++] = '\\';
            else out[i++] = **p;
        } else {
            out[i++] = **p;
        }
        (*p)++;
    }
    out[i] = '\0';
    if (**p != '"') return -1;
    (*p)++;
    return 0;
}

static int64_t parse_int(const char **p) {
    skip_whitespace(p);
    int64_t val = 0;
    int neg = 0;
    if (**p == '-') { neg = 1; (*p)++; }
    while (**p >= '0' && **p <= '9') {
        val = val * 10 + (**p - '0');
        (*p)++;
    }
    return neg ? -val : val;
}

static safetensor_dtype_t parse_dtype(const char *s) {
    if (strcmp(s, "F32") == 0) return DTYPE_F32;
    if (strcmp(s, "F16") == 0) return DTYPE_F16;
    if (strcmp(s, "BF16") == 0) return DTYPE_BF16;
    if (strcmp(s, "I32") == 0) return DTYPE_I32;
    if (strcmp(s, "I64") == 0) return DTYPE_I64;
    if (strcmp(s, "BOOL") == 0) return DTYPE_BOOL;
    return DTYPE_UNKNOWN;
}

static int parse_tensor_entry(const char **p, safetensor_t *t) {
    skip_whitespace(p);
    if (**p != '{') return -1;
    (*p)++;

    t->dtype = DTYPE_UNKNOWN;
    t->ndim = 0;
    t->data_offset = 0;
    t->data_size = 0;

    while (**p && **p != '}') {
        skip_whitespace(p);
        if (**p == ',') { (*p)++; continue; }

        char key[64];
        if (parse_string(p, key, sizeof(key)) != 0) return -1;

        skip_whitespace(p);
        if (**p != ':') return -1;
        (*p)++;
        skip_whitespace(p);

        if (strcmp(key, "dtype") == 0) {
            char dtype_str[32];
            if (parse_string(p, dtype_str, sizeof(dtype_str)) != 0) return -1;
            t->dtype = parse_dtype(dtype_str);
        } else if (strcmp(key, "shape") == 0) {
            if (**p != '[') return -1;
            (*p)++;
            t->ndim = 0;
            while (**p && **p != ']' && t->ndim < 8) {
                skip_whitespace(p);
                if (**p == ',') { (*p)++; continue; }
                t->shape[t->ndim++] = parse_int(p);
            }
            if (**p == ']') (*p)++;
        } else if (strcmp(key, "data_offsets") == 0) {
            if (**p != '[') return -1;
            (*p)++;
            skip_whitespace(p);
            size_t start = (size_t)parse_int(p);
            skip_whitespace(p);
            if (**p == ',') (*p)++;
            skip_whitespace(p);
            size_t end = (size_t)parse_int(p);
            t->data_offset = start;
            t->data_size = end - start;
            skip_whitespace(p);
            if (**p == ']') (*p)++;
        } else {
            // 跳过未知项
            if (**p == '"') {
                (*p)++;
                while (**p && **p != '"') {
                    if (**p == '\\') (*p)++;
                    if (**p) (*p)++;
                }
                if (**p == '"') (*p)++;
            } else if (**p == '[') {
                int depth = 1;
                (*p)++;
                while (**p && depth > 0) {
                    if (**p == '[') depth++;
                    else if (**p == ']') depth--;
                    (*p)++;
                }
            } else if (**p == '{') {
                int depth = 1;
                (*p)++;
                while (**p && depth > 0) {
                    if (**p == '{') depth++;
                    else if (**p == '}') depth--;
                    (*p)++;
                }
            } else {
                while (**p && **p != ',' && **p != '}') (*p)++;
            }
        }
    }
    if (**p == '}') (*p)++;
    return 0;
}

static int parse_header(safetensors_file_t *sf) {
    const char *p = sf->header_json;
    skip_whitespace(&p);
    if (*p != '{') return -1;
    p++;
    sf->num_tensors = 0;
    while (*p && *p != '}' && sf->num_tensors < SAFETENSORS_MAX_TENSORS) {
        skip_whitespace(&p);
        if (*p == ',') { p++; continue; }
        if (*p == '}') break;

        char name[256];
        if (parse_string(&p, name, sizeof(name)) != 0) return -1;
        skip_whitespace(&p);
        if (*p != ':') return -1;
        p++;

        if (strcmp(name, "__metadata__") == 0) {
            skip_whitespace(&p);
            if (*p == '{') {
                int depth = 1;
                p++;
                while (*p && depth > 0) {
                    if (*p == '{') depth++;
                    else if (*p == '}') depth--;
                    p++;
                }
            }
            continue;
        }

        safetensor_t *t = &sf->tensors[sf->num_tensors];
        snprintf(t->name, sizeof(t->name), "%s", name);
        if (parse_tensor_entry(&p, t) != 0) return -1;
        sf->num_tensors++;
    }
    return 0;
}

static safetensors_file_t *safetensors_open(const char *path) {
    int fd = open(path, O_RDONLY);
    if (fd < 0) return NULL;

    struct stat st;
    if (fstat(fd, &st) < 0) { close(fd); return NULL; }

    size_t file_size = (size_t)st.st_size;
    if (file_size < 8) { close(fd); return NULL; }

    void *data = mmap(NULL, file_size, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (data == MAP_FAILED) return NULL;

    uint64_t header_size = 0;
    memcpy(&header_size, data, 8);
    if (header_size > file_size - 8) { munmap(data, file_size); return NULL; }

    safetensors_file_t *sf = calloc(1, sizeof(safetensors_file_t));
    if (!sf) { munmap(data, file_size); return NULL; }

    sf->path = strdup(path);
    sf->data = data;
    sf->file_size = file_size;
    sf->header_size = (size_t)header_size;
    sf->header_json = malloc(header_size + 1);
    if (!sf->header_json) { free(sf->path); munmap(data, file_size); free(sf); return NULL; }

    memcpy(sf->header_json, (char *)data + 8, header_size);
    sf->header_json[header_size] = '\0';

    if (parse_header(sf) != 0) {
        free(sf->header_json);
        free(sf->path);
        munmap(data, file_size);
        free(sf);
        return NULL;
    }
    return sf;
}

static void safetensors_close(safetensors_file_t *sf) {
    if (!sf) return;
    if (sf->data) munmap(sf->data, sf->file_size);
    free(sf->path);
    free(sf->header_json);
    free(sf);
}

// FFI Objective-C 包装声明
extern void* magictts_metal_init(const char* model_dir, safetensors_file_t* sf);
extern void magictts_metal_free(void* context);
extern float* magictts_metal_synthesize(
    void* context,
    const int* tokens,
    int tokens_len,
    const float* durations,
    int steps,
    float cfg_strength,
    int* out_samples_len
);

magictts_model_t* magictts_load(const char* model_dir) {
    char safetensors_path[512];
    snprintf(safetensors_path, sizeof(safetensors_path), "%s/consolidated.safetensors", model_dir);

    safetensors_file_t *sf = safetensors_open(safetensors_path);
    if (!sf) {
        // 尝试加载 model.safetensors
        snprintf(safetensors_path, sizeof(safetensors_path), "%s/model.safetensors", model_dir);
        sf = safetensors_open(safetensors_path);
    }

    if (!sf) {
        fprintf(stderr, "Magic-TTS: failed to open safetensors file in %s\n", model_dir);
        return NULL;
    }

    magictts_model_t *model = malloc(sizeof(magictts_model_t));
    if (!model) {
        safetensors_close(sf);
        return NULL;
    }

    model->sf = sf;
    model->metal_context = magictts_metal_init(model_dir, sf);
    if (!model->metal_context) {
        safetensors_close(sf);
        free(model);
        return NULL;
    }

    return model;
}

void magictts_free(magictts_model_t* model) {
    if (!model) return;
    if (model->metal_context) {
        magictts_metal_free(model->metal_context);
    }
    if (model->sf) {
        safetensors_close(model->sf);
    }
    free(model);
}

float* magictts_synthesize(
    magictts_model_t* model,
    const int* tokens,
    int tokens_len,
    const float* durations,
    int steps,
    float cfg_strength,
    int* out_samples_len
) {
    if (!model || !model->metal_context) return NULL;
    return magictts_metal_synthesize(
        model->metal_context,
        tokens,
        tokens_len,
        durations,
        steps,
        cfg_strength,
        out_samples_len
    );
}

void magictts_free_mel(float* ptr) {
    if (ptr) {
        free(ptr);
    }
}

const safetensor_t *safetensors_find(const safetensors_file_t *sf, const char *name) {
    for (int i = 0; i < sf->num_tensors; i++) {
        if (strcmp(sf->tensors[i].name, name) == 0) {
            return &sf->tensors[i];
        }
    }
    return NULL;
}

const void *safetensors_data(const safetensors_file_t *sf, const safetensor_t *t) {
    size_t offset = 8 + sf->header_size + t->data_offset;
    return (const char *)sf->data + offset;
}

int64_t safetensor_numel(const safetensor_t *t) {
    int64_t n = 1;
    for (int i = 0; i < t->ndim; i++) {
        n *= t->shape[i];
    }
    return n;
}
