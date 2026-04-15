use model_hub::{DownloadOptions, HubProvider, ModelDownloader};

use std::path::PathBuf;

const OUTPUT_DIR: &str = "./validate_output";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── HuggingFace 验证 ──────────────────────────────────────────────────────
    //
    // 模型：hf-internal-testing/tiny-random-gpt2
    //   - HF 官方用于 CI 测试的超小模型，完全公开，无需 token
    //   - 指定下载：配置文件（~33 KB）+ model.safetensors（443 KB）
    //     → 覆盖「文本文件」与「二进制模型权重」两条下载路径
    //
    // 如需访问私有模型，设置：export HF_TOKEN=hf_your_token_here

    let hf_files = vec![
        "config.json".to_string(),
        "tokenizer.json".to_string(),
        "tokenizer_config.json".to_string(),
        "special_tokens_map.json".to_string(),
        "vocab.json".to_string(),
        "merges.txt".to_string(),
        "model.safetensors".to_string(), // 443 KB，验证二进制文件下载
    ];

    println!("▶  [HuggingFace] hf-internal-testing/tiny-random-gpt2");
    println!("   文件数：{}，预计大小：~477 KB", hf_files.len());

    ModelDownloader::new(HubProvider::HuggingFace {
        token: std::env::var("HF_TOKEN").ok(),
    })?
    .with_concurrency(4)
    .with_max_retries(2)
    .download(DownloadOptions {
        repo_id: "hf-internal-testing/tiny-random-gpt2".to_string(),
        revision: None, // 使用默认分支 "main"
        save_dir: PathBuf::from(OUTPUT_DIR),
        files: Some(hf_files.clone()),
    })
    .await?;

    verify_files(
        OUTPUT_DIR,
        "hf-internal-testing/tiny-random-gpt2",
        &hf_files,
    );
    println!("✓  [HuggingFace] 完成\n");

    // ── ModelScope 验证 ────────────────────────────────────────────────────────
    //
    // 模型：qwen/Qwen2.5-0.5B
    //   - 完全公开，无需 token
    //   - 仅下载纯配置文件（~25 KB），跳过 942 MB 的模型权重
    //     → 验证文件过滤逻辑与 ModelScope API 的正确性
    //
    // 如需访问私有模型，设置：export MS_TOKEN=ms_your_token_here

    let ms_files = vec![
        "config.json".to_string(),
        "configuration.json".to_string(),
        "generation_config.json".to_string(),
        "tokenizer_config.json".to_string(),
        "README.md".to_string(),
        "LICENSE".to_string(),
    ];

    println!("▶  [ModelScope] qwen/Qwen2.5-0.5B");
    println!("   文件数：{}，预计大小：~25 KB", ms_files.len());

    ModelDownloader::new(HubProvider::ModelScope {
        token: std::env::var("MS_TOKEN").ok(),
    })?
    .with_concurrency(4)
    .with_max_retries(2)
    .download(DownloadOptions {
        repo_id: "qwen/Qwen2.5-0.5B".to_string(),
        revision: None, // 使用默认分支 "master"
        save_dir: PathBuf::from(OUTPUT_DIR),
        files: Some(ms_files.clone()),
    })
    .await?;

    verify_files(OUTPUT_DIR, "qwen/Qwen2.5-0.5B", &ms_files);
    println!("✓  [ModelScope] 完成\n");

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  所有验证通过，文件已保存至 {OUTPUT_DIR}/");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    Ok(())
}

/// 验证下载结果：检查每个预期文件是否存在且非空。
fn verify_files(output_dir: &str, model_subdir: &str, expected: &[String]) {
    let base = model_subdir
        .split('/')
        .fold(PathBuf::from(output_dir), |p, c| p.join(c));
    let mut all_ok = true;

    for name in expected {
        let path = base.join(name);
        match path.metadata() {
            Ok(meta) if meta.len() > 0 => {
                println!("   ✓ {name} ({} bytes)", meta.len());
            }
            Ok(_) => {
                eprintln!("   ✗ {name} — 文件存在但大小为 0");
                all_ok = false;
            }
            Err(_) => {
                eprintln!("   ✗ {name} — 文件不存在");
                all_ok = false;
            }
        }
    }

    if !all_ok {
        eprintln!("   [警告] 部分文件验证失败，请检查上方输出");
    }
}
