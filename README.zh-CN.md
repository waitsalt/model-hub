<div align="right">
  <a href="README.md">English</a>
</div>

# model-hub

一个轻量级的异步 Rust 库，用于从 **Hugging Face** 和 **ModelScope** 下载机器学习模型，支持并发传输、自动重试和断点续传。

---

## 功能特性

- **多平台支持** — 开箱即用地支持 Hugging Face 和 ModelScope
- **并发下载** — 可配置并行度（默认：同时下载 4 个文件）
- **自动重试** — 遇到瞬时错误时使用指数退避策略重试（默认重试 3 次）
- **断点续传** — 通过 `Range` / `206 Partial Content` 继续中断的下载
- **文件过滤** — 通过白名单指定要下载的文件，而非下载整个仓库
- **分页支持** — 自动跟随 Hugging Face 大型仓库的 `Link: rel="next"` 响应头
- **路径穿越防护** — 在写入磁盘前对服务端返回的每个路径进行安全校验
- **自定义端点** — 通过 `HF_ENDPOINT` 环境变量覆盖 Hugging Face 基础 URL（适用于镜像站）
- **私有模型访问** — 支持通过 Bearer Token 访问受限 / 私有仓库

---

## 环境要求

| 工具  | 版本                  |
|-------|-----------------------|
| Rust  | 1.85 +（edition 2024）|
| Cargo | 随 Rust 一同安装      |

---

## 安装

在 `Cargo.toml` 中添加 `model-hub`：

```toml
[dependencies]
model-hub = { path = "path/to/model-hub" }   # 本地路径
# 发布到 crates.io 后可使用：
# model-hub = "0.1"

tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

---

## 快速开始

```rust
use model_hub::{DownloadOptions, HubProvider, ModelDownloader};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 从 Hugging Face 下载指定文件
    ModelDownloader::new(HubProvider::HuggingFace {
        token: std::env::var("HF_TOKEN").ok(),   // None → 仅公开模型
    })?
    .with_concurrency(4)
    .with_max_retries(3)
    .download(DownloadOptions {
        repo_id:  "meta-llama/Llama-2-7b-hf".to_string(),
        revision: None,                           // 使用默认分支 "main"
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

文件将保存在 `<save_dir>/<owner>/<model>/` 下，例如：
`./models/meta-llama/Llama-2-7b-hf/config.json`。

---

## API 参考

### `HubProvider`

```rust
pub enum HubProvider {
    HuggingFace { token: Option<String> },
    ModelScope   { token: Option<String> },
}
```

| 变体          | 默认分支  | 鉴权方式                        |
|---------------|----------|---------------------------------|
| `HuggingFace` | `main`   | `Authorization: Bearer <token>` |
| `ModelScope`  | `master` | `Authorization: Bearer <token>` |

---

### `ModelDownloader`

```rust
pub struct ModelDownloader { /* private */ }
```

| 方法                             | 说明                                      |
|----------------------------------|-------------------------------------------|
| `ModelDownloader::new(provider)` | 为指定平台创建下载器                      |
| `.with_concurrency(n: usize)`    | 最大并发下载数（最小为 1，默认 **4**）    |
| `.with_max_retries(n: u32)`      | 每个文件的最大重试次数（默认 **3**）      |
| `.download(options)`             | 执行下载，返回 `Result<()>`               |

---

### `DownloadOptions`

```rust
pub struct DownloadOptions {
    pub repo_id:  String,              // 例如 "meta-llama/Llama-2-7b-hf"
    pub revision: Option<String>,      // 分支、tag 或 commit hash
    pub save_dir: PathBuf,             // 本地根目录
    pub files:    Option<Vec<String>>, // None → 下载全部文件
}
```

---

## 环境变量

| 变量          | 平台          | 说明                                             |
|---------------|---------------|--------------------------------------------------|
| `HF_TOKEN`    | Hugging Face  | 用于访问私有 / 受限模型的 Bearer Token           |
| `MS_TOKEN`    | ModelScope    | 用于访问私有模型的 Bearer Token                  |
| `HF_ENDPOINT` | Hugging Face  | 覆盖基础 URL（例如 `https://hf-mirror.com`）     |

---

## 运行示例

内置的 `basic_download` 示例会从两个平台各下载一个小型公开模型，以验证完整的下载流程：

```sh
# 公开模型（无需 Token）
cargo run --example basic_download

# 携带 Token 以访问私有模型
HF_TOKEN=hf_xxx MS_TOKEN=ms_yyy cargo run --example basic_download

# 使用 Hugging Face 镜像站
HF_ENDPOINT=https://hf-mirror.com cargo run --example basic_download
```

下载的文件将保存在 `./validate_output/` 目录下。

---

## 安全说明

- **路径穿越防护** — 对服务端返回的每个路径段过滤 `..`、`.` 及绝对路径前缀，再与本地基础目录拼接。最终通过 `starts_with` 检查提供双重保障。
- **Token 安全** — Token 仅通过 HTTP 请求头传递，不会写入磁盘或出现在日志输出中。
- **语义化 User-Agent** — 客户端以 `model-hub/<version>` 标识自身，不伪造浏览器标识。

---

## 许可证

本项目基于 MIT 许可证授权。详情参见 [LICENSE](LICENSE) 文件。