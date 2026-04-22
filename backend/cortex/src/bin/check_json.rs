use serde_json::Value;
fn main() {
    let content = std::fs::read_to_string("/Users/jinliang/.omlx/models/Qwen3-TTS-12Hz-1.7B-Base/speech_tokenizer/config.json").unwrap_or("{}".to_string());
    let v: Value = serde_json::from_str(&content).unwrap();
    println!("speech_tokenizer codebook_size = {:?}", v["quantizer_config"]["codebook_size"].as_u64());
    println!("speech_tokenizer codebook_dim = {:?}", v["quantizer_config"]["codebook_dim"].as_u64());
    println!("speech_tokenizer dim = {:?}", v["quantizer_config"]["dim"].as_u64());
}
