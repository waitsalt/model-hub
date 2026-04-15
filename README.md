<div align="right">
  <a href="README.zh-CN.md">中文文档</a>
</div>

# model-hub

A lightweight, async Rust library for downloading machine learning models from **Hugging Face** and **ModelScope** with concurrent transfers, automatic retries, and resume support.

---

## Features

- **Multi-provider** — supports Hugging Face and ModelScope out of the box
- **Concurrent downloads** — configurable parallelism (default: 4 simultaneous files)
- **Automatic retry** — exponential back-off retry on transient failures (default: 3 retries)
- **Resume support** — honours `Range` / `206 Partial Content` to continue interrupted downloads
- **File filtering** — whitelist specific files instead of downloading an entire repository
- **Pagination** — follows `Link: rel="next"` headers for large Hugging Face repositories
- **Path-traversal protection** — sanitises every server-supplied path before writing to disk
- **Custom endpoint** — override the Hugging Face base URL via `HF_ENDPOINT` for mirror sites
- **Private model access** — bearer-token authentication for gated / private repositories

---

## Requirements

| Tool  | Version               |
|-------|-----------------------|
| Rust  | 1.85 + (edition 2024) |
| Cargo | bundled with Rust     |

---

## Installation

Add `model-hub` to your `Cargo.toml`:

```toml
[dependencies]
model-hub = { path = "path/to/model-hub" }   # local
# or once published to crates.io:
# model-hub = "0.1"

tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

---

## Quick Start

```rust
use model_hub::{DownloadOptions, HubProvider, ModelDownloader};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Download selected files from Hugging Face
    ModelDownloader::new(HubProvider::HuggingFace {
        token: std::env::var("HF_TOKEN").ok(),   // None → public models only
    })?
    .with_concurrency(4)
    .with_max_retries(3)
    .download(DownloadOptions {
        repo_id:  "meta-llama/Llama-2-7b-hf".to_string(),
        revision: None,                           // uses "main" by default
        save_dir: PathBuf::from("./models"),
        files:    Some(vec![
            "config.json".to_string(),
            "tokenizer.json".to_string(),
            "model.safetensors".to_string(),
        ]),
    })
    .await?;

    Ok(())
}
```

Files are saved under `<save_dir>/<owner>/<model>/`, e.g.
`./models/meta-llama/Llama-2-7b-hf/config.json`.

---

## API Reference

### `HubProvider`

```rust
pub enum HubProvider {
    HuggingFace { token: Option<String> },
    ModelScope   { token: Option<String> },
}
```

| Variant       | Default revision | Auth header                    |
|---------------|-----------------|--------------------------------|
| `HuggingFace` | `main`          | `Authorization: Bearer <token>` |
| `ModelScope`  | `master`        | `Authorization: Bearer <token>` |

---

### `ModelDownloader`

```rust
pub struct ModelDownloader { /* private */ }
```

| Method                           | Description                                          |
|----------------------------------|------------------------------------------------------|
| `ModelDownloader::new(provider)` | Create a new downloader for the given provider       |
| `.with_concurrency(n: usize)`    | Max simultaneous file downloads (min 1, default **4**) |
| `.with_max_retries(n: u32)`      | Per-file retry attempts (default **3**)              |
| `.download(options)`             | Execute the download; returns `Result<()>`           |

---

### `DownloadOptions`

```rust
pub struct DownloadOptions {
    pub repo_id:  String,              // e.g. "meta-llama/Llama-2-7b-hf"
    pub revision: Option<String>,      // branch, tag, or commit hash
    pub save_dir: PathBuf,             // local root directory
    pub files:    Option<Vec<String>>, // None → download all files
}
```

---

## Environment Variables

| Variable      | Provider      | Description                                        |
|---------------|---------------|----------------------------------------------------|
| `HF_TOKEN`    | Hugging Face  | Bearer token for private / gated models            |
| `MS_TOKEN`    | ModelScope    | Bearer token for private models                    |
| `HF_ENDPOINT` | Hugging Face  | Override base URL (e.g. `https://hf-mirror.com`)   |

---

## Running the Example

The bundled `basic_download` example downloads a tiny public model from both providers to
validate the full pipeline:

```sh
# Public models (no token required)
cargo run --example basic_download

# With tokens for private model access
HF_TOKEN=hf_xxx MS_TOKEN=ms_yyy cargo run --example basic_download

# Use a Hugging Face mirror
HF_ENDPOINT=https://hf-mirror.com cargo run --example basic_download
```

Downloaded files are placed in `./validate_output/`.

---

## Security

- **Path traversal** — every path segment returned by the server is stripped of `..`, `.`,
  and absolute-path prefixes before being joined with the local base directory. A final
  `starts_with` check provides a second layer of defence.
- **Token hygiene** — tokens are passed only in HTTP headers; they are never written to disk
  or included in log output.
- **Semantic User-Agent** — the client identifies itself as `model-hub/<version>` rather than
  spoofing a browser string.

---

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE) for details.