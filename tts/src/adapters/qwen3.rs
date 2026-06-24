use crate::config::{Qwen3Config, VoicePrompt};
use crate::engine::TtsEngine;
use anyhow::Result;
use candle_core::Device;
use qwen3_tts::{
    AudioBuffer, Language, Qwen3TTS, Speaker, SynthesisOptions, VoiceClonePrompt as QwenPrompt,
};
use std::sync::{Arc, Mutex};

pub struct Qwen3Adapter {
    model: Arc<Mutex<Qwen3TTS>>,
    prompt: Option<QwenPrompt>,
    prompt_cache_key: Option<VoicePrompt>,
    last_sample_rate: u32,
    language: Option<String>,
    speaker: Option<String>,
    voice_design_instruction: Option<String>,
    max_length: Option<usize>,
    temperature: Option<f64>,
    top_k: Option<usize>,
    top_p: Option<f64>,
    repetition_penalty: Option<f64>,
    seed: Option<u64>,
    chunk_frames: Option<usize>,
}

impl Qwen3Adapter {
    pub fn new(config: &Qwen3Config, device: &Device) -> Result<Self> {
        let model = if config.model_dir.contains('/')
            && !std::path::Path::new(&config.model_dir).exists()
        {
            let paths = qwen3_tts::ModelPaths::download(Some(config.model_dir.as_str()))?;
            Qwen3TTS::from_paths(&paths, device.clone())?
        } else {
            Qwen3TTS::from_pretrained(&config.model_dir, device.clone())?
        };
        Ok(Self::new_from_model(model, config))
    }

    pub fn new_from_model(model: Qwen3TTS, config: &Qwen3Config) -> Self {
        Self {
            model: Arc::new(Mutex::new(model)),
            prompt: None,
            prompt_cache_key: None,
            last_sample_rate: 24000,
            language: config.language.clone(),
            speaker: config.speaker.clone(),
            voice_design_instruction: config.voice_design_instruction.clone(),
            max_length: config.max_length,
            temperature: config.temperature,
            top_k: config.top_k,
            top_p: config.top_p,
            repetition_penalty: config.repetition_penalty,
            seed: config.seed,
            chunk_frames: config.chunk_frames,
        }
    }

    fn synthesis_options(&self, default_max_length: usize) -> SynthesisOptions {
        let mut options = SynthesisOptions::default();
        options.max_length = self.max_length.unwrap_or(default_max_length);
        if let Some(value) = self.temperature {
            options.temperature = value;
        }
        if let Some(value) = self.top_k {
            options.top_k = value;
        }
        if let Some(value) = self.top_p {
            options.top_p = value;
        }
        if let Some(value) = self.repetition_penalty {
            options.repetition_penalty = value;
        }
        if let Some(value) = self.chunk_frames {
            options.chunk_frames = value;
        }
        options.seed = self.seed;
        options
    }

    fn language_or(&self, default: Language) -> Language {
        let Some(language) = self.language.as_deref() else {
            return default;
        };
        match language.parse::<Language>() {
            Ok(language) => language,
            Err(e) => {
                log::warn!("Invalid Qwen3 TTS language '{}': {}", language, e);
                default
            }
        }
    }

    fn speaker_or(&self, default: Speaker) -> Speaker {
        let Some(speaker) = self.speaker.as_deref() else {
            return default;
        };
        match speaker.parse::<Speaker>() {
            Ok(speaker) => speaker,
            Err(e) => {
                log::warn!("Invalid Qwen3 TTS speaker '{}': {}", speaker, e);
                default
            }
        }
    }

    fn synthesize_standard(&self, model: &Qwen3TTS, chunk: &str) -> Result<AudioBuffer> {
        if let Some(ref prompt) = self.prompt {
            model.synthesize_voice_clone(
                chunk,
                prompt,
                self.language_or(Language::Chinese),
                Some(self.synthesis_options(1024)),
            )
        } else if self.speaker.is_some() || self.language.is_some() {
            let speaker = self.speaker_or(Speaker::Ryan);
            model.synthesize_with_voice(
                chunk,
                speaker,
                self.language_or(speaker.native_language()),
                Some(self.synthesis_options(2048)),
            )
        } else {
            model.synthesize(chunk, Some(self.synthesis_options(2048)))
        }
    }
}

#[async_trait::async_trait]
impl TtsEngine for Qwen3Adapter {
    fn cache_voice_prompt(&mut self, prompt: &VoicePrompt) -> Result<()> {
        if self.prompt.is_some() && self.prompt_cache_key.as_ref() == Some(prompt) {
            return Ok(());
        }

        if let Some(wav_path) = prompt.wav_path.as_ref().map(|p| p.replace("file://", "")) {
            let ref_audio = AudioBuffer::load(&(wav_path))?;
            let model = self.model.lock().unwrap();
            let qwen_prompt =
                model.create_voice_clone_prompt(&ref_audio, prompt.text.as_deref())?;
            self.prompt = Some(qwen_prompt);
            self.prompt_cache_key = Some(prompt.clone());
        }
        Ok(())
    }

    fn sample_rate(&self) -> usize {
        self.last_sample_rate as usize
    }

    fn synthesize_chunk(&mut self, chunk: &str) -> Result<Vec<f32>> {
        let model = self.model.lock().unwrap();
        let audio_buffer = if let Some(instruct) = self
            .voice_design_instruction
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !model.supports_voice_design() {
                log::warn!(
                    "Qwen3 voice_design_instruction configured, but this model does not support voice design; using the configured fallback mode"
                );
                self.synthesize_standard(&model, chunk)?
            } else {
                model.synthesize_voice_design(
                    chunk,
                    instruct,
                    self.language_or(Language::Chinese),
                    Some(self.synthesis_options(2048)),
                )?
            }
        } else {
            self.synthesize_standard(&model, chunk)?
        };
        self.last_sample_rate = audio_buffer.sample_rate;
        Ok(audio_buffer.samples)
    }

    fn synthesize_streaming(
        &mut self,
        text: &str,
        callback: &mut dyn FnMut(Vec<f32>) -> Result<()>,
    ) -> Result<()> {
        let model = self.model.lock().unwrap();
        let options = self.synthesis_options(2048);

        let session = if let Some(instruct) = self
            .voice_design_instruction
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if model.supports_voice_design() {
                model.synthesize_voice_design_streaming(
                    text,
                    instruct,
                    self.language_or(Language::Chinese),
                    options,
                )?
            } else {
                if let Some(ref prompt) = self.prompt {
                    model.synthesize_voice_clone_streaming(
                        text,
                        prompt,
                        self.language_or(Language::Chinese),
                        options,
                    )?
                } else {
                    let speaker = self.speaker_or(Speaker::Ryan);
                    model.synthesize_streaming(
                        text,
                        speaker,
                        self.language_or(speaker.native_language()),
                        options,
                    )?
                }
            }
        } else if let Some(ref prompt) = self.prompt {
            model.synthesize_voice_clone_streaming(
                text,
                prompt,
                self.language_or(Language::Chinese),
                options,
            )?
        } else if self.speaker.is_some() || self.language.is_some() {
            let speaker = self.speaker_or(Speaker::Ryan);
            model.synthesize_streaming(
                text,
                speaker,
                self.language_or(speaker.native_language()),
                options,
            )?
        } else {
            model.synthesize_streaming(
                text,
                Speaker::Ryan,
                Language::Chinese,
                options,
            )?
        };

        for chunk_res in session {
            let audio_buffer = chunk_res?;
            callback(audio_buffer.samples)?;
        }

        Ok(())
    }
}
