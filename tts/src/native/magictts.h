#ifndef MAGICTTS_H
#define MAGICTTS_H

#ifdef __cplusplus
extern "C" {
#endif

// 封装模型上下文信息的结构体
typedef struct magictts_model magictts_model_t;

/**
 * 从模型权重目录加载模型。通过 mmap 实现零拷贝读取。
 * @param model_dir 包含 consolidated.safetensors, vocab.txt 及其它必要配置的目录
 * @return 成功则返回模型指针，失败返回 NULL
 */
magictts_model_t* magictts_load(const char* model_dir);

/**
 * 释放模型所占用的内存和显存资源
 */
void magictts_free(magictts_model_t* model);

/**
 * 进行极致优化的语音合成。
 * @param model 模型指针
 * @param tokens 分词 Token 数组（已经经过文本控制符时序展开）
 * @param tokens_len Token 数组长度
 * @param durations Token 时值对数数组（每个 Token 有 2 个 float: [content_duration, pause_after]）
 * @param steps Euler 积分次数（通常为 32 次）
 * @param cfg_strength 引导系数（CFG，通常为 2.0）
 * @param out_samples_len 输出的原始音频 float 采样点数
 * @return 合成后的原始单声道 float 音频数组指针，该指针指向的内存需要由调用方通过 free() 释放。失败返回 NULL。
 */
float* magictts_synthesize(
    magictts_model_t* model,
    const int* tokens,
    int tokens_len,
    const float* durations,
    int steps,
    float cfg_strength,
    int* out_samples_len
);

/**
 * 释放合成接口返回的 Mel 频谱 float 数组内存
 */
void magictts_free_mel(float* ptr);

#ifdef __cplusplus
}
#endif


#endif // MAGICTTS_H
