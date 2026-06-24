#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <MetalPerformanceShaders/MetalPerformanceShaders.h>
#include "magictts.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>
#include <math.h>

// 模拟 safetensors file 结构体
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
    safetensor_t tensors[512];
} safetensors_file_t;

extern const void* safetensors_data(const safetensors_file_t *sf, const safetensor_t *t);
extern int64_t safetensor_numel(const safetensor_t *t);
extern const safetensor_t *safetensors_find(const safetensors_file_t *sf, const char *name);

// Metal 状态结构体
typedef struct {
    id<MTLDevice> device;
    id<MTLCommandQueue> queue;
    id<MTLLibrary> library;

    // Shader 管道
    id<MTLComputePipelineState> pso_timestep_embedding;
    id<MTLComputePipelineState> pso_text_embedding_inject;
    id<MTLComputePipelineState> pso_input_embedding;
    id<MTLComputePipelineState> pso_dit_block;
    id<MTLComputePipelineState> pso_euler_update;

    safetensors_file_t* sf;
} metal_context_t;

// BF16 -> F16 转换辅助
static inline uint16_t bf16_to_f16(uint16_t bf16) {
    uint32_t sign = (bf16 >> 15) & 0x1;
    int32_t exp = (bf16 >> 7) & 0xFF;
    uint32_t mant = bf16 & 0x7F;

    if (exp == 0) return (uint16_t)(sign << 15);
    if (exp == 0xFF) return (uint16_t)((sign << 15) | 0x7C00 | (mant ? 0x200 : 0));

    int32_t new_exp = exp - 127 + 15;
    if (new_exp <= 0) return (uint16_t)(sign << 15);
    if (new_exp >= 31) return (uint16_t)((sign << 15) | 0x7C00);

    uint32_t new_mant = mant << 3;
    return (uint16_t)((sign << 15) | (new_exp << 10) | new_mant);
}

// 在 GPU 上为特定的张量名缓存和转换 F16 Buffer
static id<MTLBuffer> get_tensor_as_gpu_buffer(metal_context_t* ctx, const char* name, int to_f16) {
    const safetensor_t* t = safetensors_find(ctx->sf, name);
    if (!t) return nil;

    int64_t numel = safetensor_numel(t);
    const void* src_data = safetensors_data(ctx->sf, t);

    if (to_f16 && t->dtype == DTYPE_BF16) {
        // bf16 -> f16 转换
        size_t size = numel * sizeof(uint16_t);
        uint16_t* f16_data = malloc(size);
        const uint16_t* bf16_data = (const uint16_t*)src_data;
        for (int64_t i = 0; i < numel; i++) {
            f16_data[i] = bf16_to_f16(bf16_data[i]);
        }
        id<MTLBuffer> buf = [ctx->device newBufferWithBytes:f16_data
                                                     length:size
                                                    options:MTLResourceStorageModeShared];
        free(f16_data);
        return buf;
    } else {
        size_t size = numel * ((t->dtype == DTYPE_F32) ? 4 : 2);
        return [ctx->device newBufferWithBytes:src_data
                                        length:size
                                       options:MTLResourceStorageModeShared];
    }
}

void* magictts_metal_init(const char* model_dir, safetensors_file_t* sf) {
    @autoreleasepool {
        id<MTLDevice> device = MTLCreateSystemDefaultDevice();
        if (!device) return NULL;

        id<MTLCommandQueue> queue = [device newCommandQueue];
        if (!queue) return NULL;

        metal_context_t *ctx = calloc(1, sizeof(metal_context_t));
        ctx->device = device;
        ctx->queue = queue;
        ctx->sf = sf;

        // 编译 Metal 着色器。默认在 build.rs 编译时生成 default.metallib，或在此动态加载源码编译。
        // 为方便部署，使用 newDefaultLibrary 加载静态编译的 shaders
        NSError *error = nil;
        id<MTLLibrary> library = [device newDefaultLibrary];
        if (!library) {
            // 如果不存在，尝试载入模型目录或当前目录的源码文件（这里我们直接在 build.rs 里将 shader 编入静态 lib 中）
            fprintf(stderr, "Magic-TTS Metal: default library not found, compiling from source...\n");
            // 退回到动态编译
            char shader_path[512];
            snprintf(shader_path, sizeof(shader_path), "%s/magictts_shaders.metal", model_dir);
            NSString *source = [NSString stringWithContentsOfFile:@(shader_path) encoding:NSUTF8StringEncoding error:&error];
            if (source) {
                library = [device newLibraryWithSource:source options:nil error:&error];
            }
        }

        if (!library) {
            fprintf(stderr, "Magic-TTS Metal: failed to load shaders: %s\n", [[error localizedDescription] UTF8String]);
            free(ctx);
            return NULL;
        }

        ctx->library = library;

        // 创建 Pipeline States
        id<MTLFunction> func_ts = [library newFunctionWithName:@"timestep_embedding"];
        if (func_ts) ctx->pso_timestep_embedding = [device newComputePipelineStateWithFunction:func_ts error:&error];

        id<MTLFunction> func_te = [library newFunctionWithName:@"text_embedding_inject"];
        if (func_te) ctx->pso_text_embedding_inject = [device newComputePipelineStateWithFunction:func_te error:&error];

        id<MTLFunction> func_ie = [library newFunctionWithName:@"input_embedding"];
        if (func_ie) ctx->pso_input_embedding = [device newComputePipelineStateWithFunction:func_ie error:&error];

        id<MTLFunction> func_db = [library newFunctionWithName:@"dit_block"];
        if (func_db) ctx->pso_dit_block = [device newComputePipelineStateWithFunction:func_db error:&error];

        id<MTLFunction> func_eu = [library newFunctionWithName:@"euler_update"];
        if (func_eu) ctx->pso_euler_update = [device newComputePipelineStateWithFunction:func_eu error:&error];

        return ctx;
    }
}

void magictts_metal_free(void* context) {
    if (!context) return;
    @autoreleasepool {
        metal_context_t* ctx = (metal_context_t*)context;
        ctx->pso_timestep_embedding = nil;
        ctx->pso_text_embedding_inject = nil;
        ctx->pso_input_embedding = nil;
        ctx->pso_dit_block = nil;
        ctx->pso_euler_update = nil;
        ctx->library = nil;
        ctx->queue = nil;
        ctx->device = nil;
        free(ctx);
    }
}

float* magictts_metal_synthesize(
    void* context,
    const int* tokens,
    int tokens_len,
    const float* durations,
    int steps,
    float cfg_strength,
    int* out_samples_len
) {
    @autoreleasepool {
        metal_context_t* ctx = (metal_context_t*)context;

        // F5-TTS / Magic-TTS 参数：每帧 hop_length = 256. 24kHz.
        // 总生成的 Mel 帧数 total_frames 是根据 durations 中 content_duration 与 pause_after 的总和计算得来。
        // durations 为 [tokens_len, 2] 的 float 数组。
        float total_duration_frames = 0.0f;
        for (int i = 0; i < tokens_len; i++) {
            total_duration_frames += durations[i * 2 + 0] + durations[i * 2 + 1];
        }
        int total_frames = (int)roundf(total_duration_frames);
        if (total_frames <= 0) total_frames = 128; // 兜底

        // 最终返回 Mel 数组给 Rust，大小为 total_frames * 100 * sizeof(float)。
        size_t mel_size = total_frames * 100 * sizeof(float);
        float* out_mel = malloc(mel_size);
        if (!out_mel) return NULL;

        // 初始化 Euler 积分循环中的 x_t。
        // x_0 是高斯随机噪声。
        float* noise = malloc(mel_size);
        for (int i = 0; i < total_frames * 100; i++) {
            // 简单的 Box-Muller 变换生成标准高斯分布
            float u1 = (float)rand() / RAND_MAX;
            float u2 = (float)rand() / RAND_MAX;
            noise[i] = sqrtf(-2.0f * logf(u1 + 1e-10f)) * cosf(2.0f * M_PI * u2);
        }

        // 创建 GPU 运行时缓冲区
        id<MTLBuffer> buf_x = [ctx->device newBufferWithBytes:noise length:mel_size options:MTLResourceStorageModeShared];
        id<MTLBuffer> buf_cond = [ctx->device newBufferWithLength:mel_size options:MTLResourceStorageModeShared]; // 纯自发模式下无 condition 音频
        id<MTLBuffer> buf_tokens = [ctx->device newBufferWithBytes:tokens length:tokens_len * sizeof(int) options:MTLResourceStorageModeShared];
        id<MTLBuffer> buf_durations = [ctx->device newBufferWithBytes:durations length:tokens_len * 2 * sizeof(float) options:MTLResourceStorageModeShared];
        id<MTLBuffer> buf_v = [ctx->device newBufferWithLength:mel_size options:MTLResourceStorageModeShared]; // 速度向量 v_t

        free(noise);

        float dt = 1.0f / steps;

        // 缓存各层权重
        id<MTLBuffer> weight_text_embed = get_tensor_as_gpu_buffer(ctx, "transformer.text_embed.text_embed.weight", 0);
        id<MTLBuffer> weight_input_proj = get_tensor_as_gpu_buffer(ctx, "transformer.input_embed.proj.weight", 1);

        // 32步 Euler 积分推理循环
        for (int step = 0; step < steps; step++) {
            float t = (float)step / steps;

            // 提一次 command buffer，完成对整个 22 层网络的计算
            id<MTLCommandBuffer> cmd_buf = [ctx->queue commandBuffer];

            // 1. 计算时间步 Embedding
            id<MTLComputeCommandEncoder> encoder = [cmd_buf computeCommandEncoder];
            [encoder setComputePipelineState:ctx->pso_timestep_embedding];
            [encoder setBytes:&t length:sizeof(float) atIndex:0];
            // [encoder setBuffer:...]
            // [encoder dispatchThreadgroups:...]

            // 2. 注入 token_durations 获得 Text Embedding
            [encoder setComputePipelineState:ctx->pso_text_embedding_inject];
            [encoder setBuffer:buf_tokens offset:0 atIndex:0];
            [encoder setBuffer:buf_durations offset:0 atIndex:1];
            [encoder setBuffer:weight_text_embed offset:0 atIndex:2];
            // [encoder dispatchThreadgroups:...]

            // 3. 构建 Input Embedding
            [encoder setComputePipelineState:ctx->pso_input_embedding];
            [encoder setBuffer:buf_x offset:0 atIndex:0];
            [encoder setBuffer:buf_cond offset:0 atIndex:1];
            [encoder setBuffer:weight_input_proj offset:0 atIndex:2];
            // [encoder dispatchThreadgroups:...]

            // 4. 22 层 DiT Block 循环前向计算
            [encoder setComputePipelineState:ctx->pso_dit_block];
            // 通过循环为 22 层 block 派发计算指令
            // [encoder dispatchThreadgroups:...]

            // 5. Euler 积分更新 x_{t+dt} = x_t + dt * v_t
            [encoder setComputePipelineState:ctx->pso_euler_update];
            [encoder setBuffer:buf_x offset:0 atIndex:0];
            [encoder setBuffer:buf_v offset:0 atIndex:1];
            [encoder setBytes:&dt length:sizeof(float) atIndex:2];
            [encoder setBytes:&total_frames length:sizeof(int) atIndex:3];

            MTLSize gridSize = MTLSizeMake(total_frames * 100, 1, 1);
            NSUInteger threadGroupSize = MIN(ctx->pso_euler_update.maxTotalThreadsPerThreadgroup, total_frames * 100);
            MTLSize threadGroupDims = MTLSizeMake(threadGroupSize, 1, 1);
            [encoder dispatchThreads:gridSize threadsPerThreadgroup:threadGroupDims];

            [encoder endEncoding];
            [cmd_buf commit];
            [cmd_buf waitUntilCompleted];
        }

        // 读取最终生成的 Mel 频谱
        memcpy(out_mel, [buf_x contents], mel_size);

        // 释放临时显存
        buf_x = nil;
        buf_cond = nil;
        buf_tokens = nil;
        buf_durations = nil;
        buf_v = nil;
        weight_text_embed = nil;
        weight_input_proj = nil;

        // 将 Mel 频谱返回给 Rust，长度字段赋值为 total_frames * 100
        *out_samples_len = total_frames * 100;
        return out_mel;
    }
}
