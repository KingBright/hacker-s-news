use candle_core::{Device, Module, Result, Tensor, D};
use candle_nn::{Embedding, LayerNorm, Linear, VarBuilder};

// ==========================================
// -1. 零可学习参数 LayerNorm (elementwise_affine=False)
// ==========================================
pub struct LayerNormNoAffine {
    eps: f64,
}

impl LayerNormNoAffine {
    pub fn new(eps: f64) -> Self {
        Self { eps }
    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let mean = x.mean_keepdim(D::Minus1)?;
        let mean_sq = x.sqr()?.mean_keepdim(D::Minus1)?;
        let sq_mean = mean.sqr()?;
        let var = mean_sq.sub(&sq_mean)?;
        let norm = x
            .broadcast_sub(&mean)?
            .broadcast_div(&var.affine(1.0, self.eps)?.sqrt()?)?;
        Ok(norm)
    }
}

// ==========================================
// 0. RoPE 旋转位置编码支持
// ==========================================
pub struct RotaryEmbedding {
    dim: usize,
}

impl RotaryEmbedding {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }

    pub fn forward_from_seq_len(
        &self,
        seq_len: usize,
        device: &Device,
    ) -> Result<(Tensor, Tensor)> {
        let half_dim = self.dim / 2;
        let inv_freq: Vec<f32> = (0..half_dim)
            .map(|i| 1.0f32 / (10000.0f32.powf((i * 2) as f32 / self.dim as f32)))
            .collect();
        let inv_freq = Tensor::from_slice(&inv_freq, half_dim, device)?;
        let t = Tensor::arange(0.0f32, seq_len as f32, device)?;
        // freqs: [seq_len, half_dim]
        let freqs = t.unsqueeze(1)?.matmul(&inv_freq.unsqueeze(0)?)?;
        let freqs_cos = freqs.cos()?;
        let freqs_sin = freqs.sin()?;
        Ok((freqs_cos, freqs_sin))
    }
}

fn rot_half(x: &Tensor) -> Result<Tensor> {
    let last_dim = x.dim(D::Minus1)?;
    let half_dim = last_dim / 2;
    let x1 = x.narrow(D::Minus1, 0, half_dim)?;
    let x2 = x.narrow(D::Minus1, half_dim, half_dim)?;
    Tensor::cat(&[&x2.neg()?, &x1], D::Minus1)
}

fn apply_rotary_pos_emb(x: &Tensor, cos: &Tensor, sin: &Tensor) -> Result<Tensor> {
    let n = x.dim(1)?;
    let dim_head = x.dim(3)?;

    let cos_emb = Tensor::cat(&[cos, cos], D::Minus1)?.reshape((1, n, 1, dim_head))?;
    let sin_emb = Tensor::cat(&[sin, sin], D::Minus1)?.reshape((1, n, 1, dim_head))?;

    let x_rot = rot_half(x)?;
    let out = x
        .broadcast_mul(&cos_emb)?
        .add(&x_rot.broadcast_mul(&sin_emb)?)?;
    Ok(out)
}

// ==========================================
// 1. GRN (Global Response Normalization)
// ==========================================
pub struct Grn {
    gamma: Tensor,
    beta: Tensor,
}

impl Grn {
    pub fn new(dim: usize, vb: VarBuilder) -> Result<Self> {
        let gamma = vb.get((1, 1, dim), "gamma")?;
        let beta = vb.get((1, 1, dim), "beta")?;
        Ok(Self { gamma, beta })
    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let gx = x.sqr()?.sum_keepdim(1)?.sqrt()?;
        let gx_mean = gx.mean_keepdim(D::Minus1)?;
        let nx = gx.broadcast_div(&gx_mean.affine(1.0, 1e-6)?)?;
        let out = self
            .gamma
            .broadcast_mul(&x.broadcast_mul(&nx)?)?
            .broadcast_add(&self.beta)?
            .add(x)?;
        Ok(out)
    }
}

// ==========================================
// 2. ConvNeXt-V2 Block
// ==========================================
pub struct ConvNeXtV2Block {
    dwconv: candle_nn::Conv1d,
    norm: LayerNorm,
    pwconv1: Linear,
    grn: Grn,
    pwconv2: Linear,
}

impl ConvNeXtV2Block {
    pub fn new(dim: usize, intermediate_dim: usize, vb: VarBuilder) -> Result<Self> {
        let cfg = candle_nn::Conv1dConfig {
            padding: 3,
            groups: dim,
            dilation: 1,
            ..Default::default()
        };
        let dwconv = candle_nn::conv1d(dim, dim, 7, cfg, vb.pp("dwconv"))?;
        let norm = candle_nn::layer_norm(dim, 1e-6, vb.pp("norm"))?;
        let pwconv1 = candle_nn::linear(dim, intermediate_dim, vb.pp("pwconv1"))?;
        let grn = Grn::new(intermediate_dim, vb.pp("grn"))?;
        let pwconv2 = candle_nn::linear(intermediate_dim, dim, vb.pp("pwconv2"))?;
        Ok(Self {
            dwconv,
            norm,
            pwconv1,
            grn,
            pwconv2,
        })
    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let residual = x.clone();
        let mut x = x.transpose(1, 2)?.contiguous()?;
        x = self.dwconv.forward(&x)?;
        let mut x = x.transpose(1, 2)?.contiguous()?;
        x = self.norm.forward(&x)?;
        x = self.pwconv1.forward(&x)?.silu()?;
        x = self.grn.forward(&x)?;
        x = self.pwconv2.forward(&x)?;
        x.add(&residual)
    }
}

// ==========================================
// 3. TextEmbedding
// ==========================================
pub struct TextEmbedding {
    text_embed: Embedding,
    use_duration_condition: bool,
    duration_log_scale: f64,
    content_duration_mlp_0: Option<Linear>,
    content_duration_mlp_2: Option<Linear>,
    pause_duration_mlp_0: Option<Linear>,
    pause_duration_mlp_2: Option<Linear>,
    alpha_content: Option<Tensor>,
    alpha_pause: Option<Tensor>,
    text_blocks: Vec<ConvNeXtV2Block>,
}

impl TextEmbedding {
    pub fn new(
        text_num_embeds: usize,
        text_dim: usize,
        conv_layers: usize,
        use_duration_condition: bool,
        duration_log_scale: f64,
        vb: VarBuilder,
    ) -> Result<Self> {
        let text_embed = Embedding::new(
            vb.get((text_num_embeds + 1, text_dim), "text_embed.weight")?,
            text_dim,
        );

        let (cd_mlp0, cd_mlp2, pd_mlp0, pd_mlp2, a_content, a_pause) = if use_duration_condition {
            let cd0 = candle_nn::linear(1, text_dim, vb.pp("content_duration_mlp.0"))?;
            let cd2 = candle_nn::linear(text_dim, text_dim, vb.pp("content_duration_mlp.2"))?;
            let pd0 = candle_nn::linear(1, text_dim, vb.pp("pause_duration_mlp.0"))?;
            let pd2 = candle_nn::linear(text_dim, text_dim, vb.pp("pause_duration_mlp.2"))?;
            let ac = vb.get(1, "alpha_content")?;
            let ap = vb.get(1, "alpha_pause")?;
            (
                Some(cd0),
                Some(cd2),
                Some(pd0),
                Some(pd2),
                Some(ac),
                Some(ap),
            )
        } else {
            (None, None, None, None, None, None)
        };

        let mut text_blocks = Vec::new();
        let blocks_vb = vb.pp("text_blocks");
        for i in 0..conv_layers {
            let block = ConvNeXtV2Block::new(text_dim, text_dim * 2, blocks_vb.pp(i))?;
            text_blocks.push(block);
        }

        Ok(Self {
            text_embed,
            use_duration_condition,
            duration_log_scale,
            content_duration_mlp_0: cd_mlp0,
            content_duration_mlp_2: cd_mlp2,
            pause_duration_mlp_0: pd_mlp0,
            pause_duration_mlp_2: pd_mlp2,
            alpha_content: a_content,
            alpha_pause: a_pause,
            text_blocks,
        })
    }

    pub fn forward(
        &self,
        text: &Tensor,
        seq_len: usize,
        token_durations: Option<&Tensor>,
    ) -> Result<Tensor> {
        let mut text_features = self.text_embed.forward(text)?;

        if self.use_duration_condition {
            if let Some(durations) = token_durations {
                let mut durations = durations.to_dtype(text_features.dtype())?;
                if durations.dim(1)? < seq_len {
                    durations = durations.pad_with_zeros(1, 0, seq_len - durations.dim(1)?)?;
                } else {
                    durations = durations.narrow(1, 0, seq_len)?;
                }

                let content_dur = durations.narrow(D::Minus1, 0, 1)?;
                let pause_dur = durations.narrow(D::Minus1, 1, 1)?;
                let zero = Tensor::zeros_like(&content_dur)?;

                let c_in = content_dur.affine(self.duration_log_scale, 1.0)?.log()?;
                let c_res = self.content_duration_mlp_2.as_ref().unwrap().forward(
                    &self
                        .content_duration_mlp_0
                        .as_ref()
                        .unwrap()
                        .forward(&c_in)?
                        .silu()?,
                )?;
                let c_zero_res = self.content_duration_mlp_2.as_ref().unwrap().forward(
                    &self
                        .content_duration_mlp_0
                        .as_ref()
                        .unwrap()
                        .forward(&zero)?
                        .silu()?,
                )?;
                let content_residual = c_res.sub(&c_zero_res)?;

                let p_in = pause_dur.affine(self.duration_log_scale, 1.0)?.log()?;
                let p_res = self.pause_duration_mlp_2.as_ref().unwrap().forward(
                    &self
                        .pause_duration_mlp_0
                        .as_ref()
                        .unwrap()
                        .forward(&p_in)?
                        .silu()?,
                )?;
                let p_zero_res = self.pause_duration_mlp_2.as_ref().unwrap().forward(
                    &self
                        .pause_duration_mlp_0
                        .as_ref()
                        .unwrap()
                        .forward(&zero)?
                        .silu()?,
                )?;
                let pause_residual = p_res.sub(&p_zero_res)?;

                let a_c = self.alpha_content.as_ref().unwrap();
                let a_p = self.alpha_pause.as_ref().unwrap();
                let duration_res = content_residual
                    .broadcast_mul(a_c)?
                    .add(&pause_residual.broadcast_mul(a_p)?)?;

                text_features = text_features.add(&duration_res)?;
            }
        }

        for block in &self.text_blocks {
            text_features = block.forward(&text_features)?;
        }

        Ok(text_features)
    }
}

// ==========================================
// 4. InputEmbedding
// ==========================================
pub struct ConvPositionEmbedding {
    conv1: candle_nn::Conv1d,
    conv2: candle_nn::Conv1d,
}

impl ConvPositionEmbedding {
    pub fn new(dim: usize, vb: VarBuilder) -> Result<Self> {
        let cfg = candle_nn::Conv1dConfig {
            padding: 15,
            groups: 16,
            dilation: 1,
            ..Default::default()
        };
        let conv1 = candle_nn::conv1d(dim, dim, 31, cfg, vb.pp("conv1d.0"))?;
        let conv2 = candle_nn::conv1d(dim, dim, 31, cfg, vb.pp("conv1d.2"))?;
        Ok(Self { conv1, conv2 })
    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let mut h = x.transpose(1, 2)?.contiguous()?;
        h = self.conv1.forward(&h)?;
        h = h.mul(&h.tanh()?)?;
        h = self.conv2.forward(&h)?;
        h = h.mul(&h.tanh()?)?;
        let out = h.transpose(1, 2)?.contiguous()?;
        Ok(out)
    }
}

pub struct InputEmbedding {
    proj: Linear,
    conv_pos_embed: ConvPositionEmbedding,
}

impl InputEmbedding {
    pub fn new(mel_dim: usize, text_dim: usize, out_dim: usize, vb: VarBuilder) -> Result<Self> {
        let proj = candle_nn::linear(mel_dim * 2 + text_dim, out_dim, vb.pp("proj"))?;
        let conv_pos_embed = ConvPositionEmbedding::new(out_dim, vb.pp("conv_pos_embed"))?;
        Ok(Self {
            proj,
            conv_pos_embed,
        })
    }

    pub fn forward(&self, x: &Tensor, cond: &Tensor, text_embed: &Tensor) -> Result<Tensor> {
        let combined = Tensor::cat(&[x, cond, text_embed], D::Minus1)?;
        let mut h = self.proj.forward(&combined)?;
        let pos_h = self.conv_pos_embed.forward(&h)?;
        h = h.add(&pos_h)?;
        Ok(h)
    }
}

// ==========================================
// 5. AdaLayerNormZero / Final
// ==========================================
pub struct AdaLayerNormZero {
    linear: Linear,
    norm: LayerNormNoAffine,
}

impl AdaLayerNormZero {
    pub fn new(dim: usize, vb: VarBuilder) -> Result<Self> {
        let linear = candle_nn::linear(dim, dim * 6, vb.pp("linear"))?;
        let norm = LayerNormNoAffine::new(1e-6);
        Ok(Self { linear, norm })
    }

    pub fn forward(
        &self,
        x: &Tensor,
        emb: &Tensor,
    ) -> Result<(Tensor, Tensor, Tensor, Tensor, Tensor)> {
        let emb_proj = self.linear.forward(&emb.silu()?)?;
        let chunk_size = emb_proj.dim(1)? / 6;
        let shift_msa = emb_proj.narrow(1, 0, chunk_size)?;
        let scale_msa = emb_proj.narrow(1, chunk_size, chunk_size)?;
        let gate_msa = emb_proj.narrow(1, chunk_size * 2, chunk_size)?;
        let shift_mlp = emb_proj.narrow(1, chunk_size * 3, chunk_size)?;
        let scale_mlp = emb_proj.narrow(1, chunk_size * 4, chunk_size)?;
        let gate_mlp = emb_proj.narrow(1, chunk_size * 5, chunk_size)?;

        let mut h = self.norm.forward(x)?;
        let scale_msa_expanded = scale_msa.unsqueeze(1)?;
        let shift_msa_expanded = shift_msa.unsqueeze(1)?;
        h = h
            .broadcast_mul(&scale_msa_expanded.affine(1.0, 1.0)?)?
            .broadcast_add(&shift_msa_expanded)?;

        Ok((h, gate_msa, shift_mlp, scale_mlp, gate_mlp))
    }
}

pub struct AdaLayerNormZeroFinal {
    linear: Linear,
    norm: LayerNormNoAffine,
}

impl AdaLayerNormZeroFinal {
    pub fn new(dim: usize, vb: VarBuilder) -> Result<Self> {
        let linear = candle_nn::linear(dim, dim * 2, vb.pp("linear"))?;
        let norm = LayerNormNoAffine::new(1e-6);
        Ok(Self { linear, norm })
    }

    pub fn forward(&self, x: &Tensor, emb: &Tensor) -> Result<Tensor> {
        let emb_proj = self.linear.forward(&emb.silu()?)?;
        let chunk_size = emb_proj.dim(1)? / 2;
        let scale = emb_proj.narrow(1, 0, chunk_size)?;
        let shift = emb_proj.narrow(1, chunk_size, chunk_size)?;

        let mut h = self.norm.forward(x)?;
        let scale_expanded = scale.unsqueeze(1)?;
        let shift_expanded = shift.unsqueeze(1)?;
        h = h
            .broadcast_mul(&scale_expanded.affine(1.0, 1.0)?)?
            .broadcast_add(&shift_expanded)?;
        Ok(h)
    }
}

// ==========================================
// 6. Attention & FeedForward
// ==========================================
pub struct FeedForward {
    project_in: Linear,
    project_out: Linear,
}

impl FeedForward {
    pub fn new(dim: usize, mult: usize, vb: VarBuilder) -> Result<Self> {
        let inner_dim = dim * mult;
        let project_in = candle_nn::linear(dim, inner_dim, vb.pp("ff.0.0"))?;
        let project_out = candle_nn::linear(inner_dim, dim, vb.pp("ff.2"))?;
        Ok(Self {
            project_in,
            project_out,
        })
    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let h = self.project_in.forward(x)?.gelu_erf()?;
        self.project_out.forward(&h)
    }
}

pub struct Attention {
    to_q: Linear,
    to_k: Linear,
    to_v: Linear,
    to_out: Linear,
    heads: usize,
    dim_head: usize,
}

impl Attention {
    pub fn new(dim: usize, heads: usize, dim_head: usize, vb: VarBuilder) -> Result<Self> {
        let inner_dim = heads * dim_head;
        let to_q = candle_nn::linear(dim, inner_dim, vb.pp("to_q"))?;
        let to_k = candle_nn::linear(dim, inner_dim, vb.pp("to_k"))?;
        let to_v = candle_nn::linear(dim, inner_dim, vb.pp("to_v"))?;
        let to_out = candle_nn::linear(inner_dim, dim, vb.pp("to_out.0"))?;
        Ok(Self {
            to_q,
            to_k,
            to_v,
            to_out,
            heads,
            dim_head,
        })
    }

    pub fn forward(&self, x: &Tensor, cos: &Tensor, sin: &Tensor) -> Result<Tensor> {
        let (b, n, _) = x.dims3()?;
        let q = self
            .to_q
            .forward(x)?
            .reshape((b, n, self.heads, self.dim_head))?;
        let k = self
            .to_k
            .forward(x)?
            .reshape((b, n, self.heads, self.dim_head))?;
        let v = self
            .to_v
            .forward(x)?
            .reshape((b, n, self.heads, self.dim_head))?;

        let q = apply_rotary_pos_emb(&q, cos, sin)?;
        let k = apply_rotary_pos_emb(&k, cos, sin)?;

        let q = q.transpose(1, 2)?.contiguous()?;
        let k = k.transpose(1, 2)?.contiguous()?;
        let v = v.transpose(1, 2)?.contiguous()?;

        let scale = 1.0f64 / (self.dim_head as f64).sqrt();
        let scores = q.matmul(&k.transpose(2, 3)?)?.affine(scale, 0.0)?;
        let attn_weights = candle_nn::ops::softmax(&scores, D::Minus1)?;

        let context = attn_weights.matmul(&v)?;
        let context = context.transpose(1, 2)?.contiguous()?.reshape((b, n, ()))?;
        self.to_out.forward(&context)
    }
}

// ==========================================
// 7. DiTBlock
// ==========================================
pub struct DiTBlock {
    attn_norm: AdaLayerNormZero,
    attn: Attention,
    ff_norm: LayerNormNoAffine,
    ff: FeedForward,
}

impl DiTBlock {
    pub fn new(
        dim: usize,
        heads: usize,
        dim_head: usize,
        ff_mult: usize,
        vb: VarBuilder,
    ) -> Result<Self> {
        let attn_norm = AdaLayerNormZero::new(dim, vb.pp("attn_norm"))?;
        let attn = Attention::new(dim, heads, dim_head, vb.pp("attn"))?;
        let ff_norm = LayerNormNoAffine::new(1e-6);
        let ff = FeedForward::new(dim, ff_mult, vb.pp("ff"))?;
        Ok(Self {
            attn_norm,
            attn,
            ff_norm,
            ff,
        })
    }

    pub fn forward(&self, x: &Tensor, t: &Tensor, cos: &Tensor, sin: &Tensor) -> Result<Tensor> {
        let (norm, gate_msa, shift_mlp, scale_mlp, gate_mlp) = self.attn_norm.forward(x, t)?;
        let attn_output = self.attn.forward(&norm, cos, sin)?;
        let mut x = x.add(&attn_output.broadcast_mul(&gate_msa.unsqueeze(1)?)?)?;

        let mut ff_norm_x = self.ff_norm.forward(&x)?;
        ff_norm_x = ff_norm_x
            .broadcast_mul(&scale_mlp.unsqueeze(1)?.affine(1.0, 1.0)?)?
            .broadcast_add(&shift_mlp.unsqueeze(1)?)?;

        let ff_output = self.ff.forward(&ff_norm_x)?;
        x = x.add(&ff_output.broadcast_mul(&gate_mlp.unsqueeze(1)?)?)?;
        Ok(x)
    }
}

// ==========================================
// 8. TimestepEmbedding
// ==========================================
pub struct SinusoidalPosEmb {
    dim: usize,
}

impl SinusoidalPosEmb {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let half_dim = self.dim / 2;
        let inv_freq: Vec<f32> = (0..half_dim)
            .map(|i| 1.0f32 / (10000.0f32.powf((i * 2) as f32 / self.dim as f32)))
            .collect();
        let inv_freq = Tensor::from_slice(&inv_freq, half_dim, x.device())?;
        let scaled_x = x.unsqueeze(1)?.affine(1000.0, 0.0)?;
        let freqs = scaled_x.matmul(&inv_freq.unsqueeze(0)?)?;
        let emb = Tensor::cat(&[freqs.sin()?, freqs.cos()?], D::Minus1)?;
        Ok(emb)
    }
}

pub struct TimestepEmbedding {
    time_embed: SinusoidalPosEmb,
    time_mlp_0: Linear,
    time_mlp_2: Linear,
}

impl TimestepEmbedding {
    pub fn new(dim: usize, freq_embed_dim: usize, vb: VarBuilder) -> Result<Self> {
        let time_embed = SinusoidalPosEmb::new(freq_embed_dim);
        let time_mlp_0 = candle_nn::linear(freq_embed_dim, dim, vb.pp("time_mlp.0"))?;
        let time_mlp_2 = candle_nn::linear(dim, dim, vb.pp("time_mlp.2"))?;
        Ok(Self {
            time_embed,
            time_mlp_0,
            time_mlp_2,
        })
    }

    pub fn forward(&self, timestep: &Tensor) -> Result<Tensor> {
        let time_hidden = self.time_embed.forward(timestep)?;
        let time = self
            .time_mlp_2
            .forward(&self.time_mlp_0.forward(&time_hidden)?.silu()?)?;
        Ok(time)
    }
}

// ==========================================
// 9. DiT 主骨干网络
// ==========================================
pub struct DiT {
    time_embed: TimestepEmbedding,
    text_embed: TextEmbedding,
    input_embed: InputEmbedding,
    rotary_embed: RotaryEmbedding,
    transformer_blocks: Vec<DiTBlock>,
    norm_out: AdaLayerNormZeroFinal,
    proj_out: Linear,
    proj_out_ln_sig: Linear,
}

impl DiT {
    pub fn new(
        dim: usize,
        depth: usize,
        heads: usize,
        dim_head: usize,
        ff_mult: usize,
        mel_dim: usize,
        text_num_embeds: usize,
        text_dim: usize,
        conv_layers: usize,
        duration_condition: bool,
        duration_log_scale: f64,
        vb: VarBuilder,
    ) -> Result<Self> {
        let time_embed = TimestepEmbedding::new(dim, 256, vb.pp("time_embed"))?;
        let text_embed = TextEmbedding::new(
            text_num_embeds,
            text_dim,
            conv_layers,
            duration_condition,
            duration_log_scale,
            vb.pp("text_embed"),
        )?;
        let input_embed = InputEmbedding::new(mel_dim, text_dim, dim, vb.pp("input_embed"))?;
        let rotary_embed = RotaryEmbedding::new(dim_head);

        let mut transformer_blocks = Vec::new();
        let blocks_vb = vb.pp("transformer_blocks");
        for i in 0..depth {
            let block = DiTBlock::new(dim, heads, dim_head, ff_mult, blocks_vb.pp(i))?;
            transformer_blocks.push(block);
        }

        let norm_out = AdaLayerNormZeroFinal::new(dim, vb.pp("norm_out"))?;
        let proj_out = candle_nn::linear(dim, mel_dim, vb.pp("proj_out"))?;
        let proj_out_ln_sig = candle_nn::linear(dim, mel_dim, vb.pp("proj_out_ln_sig"))?;

        Ok(Self {
            time_embed,
            text_embed,
            input_embed,
            rotary_embed,
            transformer_blocks,
            norm_out,
            proj_out,
            proj_out_ln_sig,
        })
    }

    pub fn forward(
        &self,
        x: &Tensor,
        cond: &Tensor,
        text: &Tensor,
        time: &Tensor,
        token_durations: Option<&Tensor>,
    ) -> Result<Tensor> {
        let seq_len = x.dim(1)?;
        let t = self.time_embed.forward(time)?;
        let text_embed = self.text_embed.forward(text, seq_len, token_durations)?;
        let mut h = self.input_embed.forward(x, cond, &text_embed)?;

        let (cos, sin) = self
            .rotary_embed
            .forward_from_seq_len(seq_len, x.device())?;

        for block in &self.transformer_blocks {
            h = block.forward(&h, &t, &cos, &sin)?;
        }

        let h_norm = self.norm_out.forward(&h, &t)?;
        let pred_mu = self.proj_out.forward(&h_norm)?;
        let pred_ln_sig = self.proj_out_ln_sig.forward(&h_norm)?;

        let randn = pred_mu.randn_like(0.0, 1.0)?;
        let out = pred_mu.add(&randn.mul(&pred_ln_sig.exp()?)?)?;
        Ok(out)
    }
}
