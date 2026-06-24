use crate::adapters::magictts::model::DiT;
use candle_core::Result;
use candle_core::{Device, Tensor};

pub struct UnifiedCFM {
    transformer: DiT,
}

impl UnifiedCFM {
    pub fn new(transformer: DiT) -> Self {
        Self { transformer }
    }

    pub fn solve_euler(
        &self,
        cond: &Tensor,         // [1, cond_len, mel_dim]
        text: &Tensor,         // [1, text_len]
        durations: &Tensor,    // [1, text_len, 2]
        total_duration: usize, // 预测的总帧数
        steps: usize,
        cfg_strength: f64,
        device: &Device,
    ) -> Result<Tensor> {
        let dtype = cond.dtype();

        // 1. 初始化噪声 y0: [1, total_duration, 100]
        let mut x =
            Tensor::randn(0.0f32, 1.0, (1, total_duration, 100), device)?.to_dtype(dtype)?;

        // 2. 构造 cond padding 到 total_duration
        let cond_seq_len = cond.dim(1)?;
        let mut step_cond = cond.clone();
        if cond_seq_len < total_duration {
            step_cond = step_cond.pad_with_zeros(1, 0, total_duration - cond_seq_len)?;
        } else {
            step_cond = step_cond.narrow(1, 0, total_duration)?;
        }

        // 3. Euler 积分去噪循环
        let dt = 1.0f64 / steps as f64;

        for step in 0..steps {
            let t_val = step as f64 * dt;
            let t_tensor = Tensor::from_slice(&[t_val as f32], 1, device)?.to_dtype(dtype)?;

            // 条件预测 (Conditional Flow)
            let pred =
                self.transformer
                    .forward(&x, &step_cond, text, &t_tensor, Some(durations))?;

            let v = if cfg_strength > 1e-5 {
                // 无条件预测 (Unconditional Flow)
                let null_cond = Tensor::zeros_like(&step_cond)?;
                let null_text = Tensor::zeros_like(text)?;
                let null_pred = self
                    .transformer
                    .forward(&x, &null_cond, &null_text, &t_tensor, None)?;
                // pred + (pred - null_pred) * cfg_strength
                pred.add(&pred.sub(&null_pred)?.affine(cfg_strength, 0.0)?)?
            } else {
                pred
            };

            // x = x + dt * v
            x = x.add(&v.affine(dt, 0.0)?)?;
        }

        // 保留参考音频部分以对齐 E2-TTS/F5-TTS 的行为 (preserve_cond_audio_in_output)
        if cond_seq_len < total_duration {
            let cond_mask = Tensor::cat(
                &[
                    Tensor::ones((1, cond_seq_len, 100), dtype, device)?,
                    Tensor::zeros((1, total_duration - cond_seq_len, 100), dtype, device)?,
                ],
                1,
            )?;
            // out = cond * mask + x * (1 - mask)
            let one_minus_mask = cond_mask.affine(-1.0, 1.0)?;
            x = step_cond.mul(&cond_mask)?.add(&x.mul(&one_minus_mask)?)?;
        }

        Ok(x)
    }
}
