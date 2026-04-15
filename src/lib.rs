use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

// ── Provider ──────────────────────────────────────────────────────────────────

/// 下载平台及鉴权信息。
#[derive(Debug, Clone)]
pub enum HubProvider {
    /// Hugging Face 平台，可选传入 HF Token。
    HuggingFace { token: Option<String> },
    /// ModelScope 平台，可选传入 AccessToken。
    ModelScope { token: Option<String> },
}

impl HubProvider {
    /// 统一获取 token，消除重复分支逻辑。
    fn token(&self) -> Option<&str> {
        match self {
            Self::HuggingFace { token } | Self::ModelScope { token } => token.as_deref(),
        }
    }

    /// 各平台默认分支名称。
    fn default_revision(&self) -> &'static str {
        match self {
            Self::HuggingFace { .. } => "main",
            Self::ModelScope { .. } => "master",
        }
    }
}

// ── 内部数据结构 ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct HfFile {
    path: String,
    size: u64,
    r#type: String,
}

#[derive(Debug, Deserialize)]
struct MsResponse {
    #[serde(rename = "Success")]
    success: bool,
    #[serde(rename = "Data")]
    data: Option<MsData>,
}

#[derive(Debug, Deserialize)]
struct MsData {
    #[serde(rename = "Files")]
    files: Vec<MsFile>,
}

#[derive(Debug, Deserialize)]
struct MsFile {
    #[serde(rename = "Path")]
    path: String,
    #[serde(rename = "Size")]
    size: u64,
    #[serde(rename = "Type")]
    r#type: String,
}

/// 平台无关的统一文件描述，需要 Clone 以便在 retry 闭包中复用。
#[derive(Clone)]
struct UnifiedFile {
    path: String,
    size: u64,
    download_url: String,
}

// ── 公开 API ───────────────────────────────────────────────────────────────────

/// 单次下载请求的参数。
pub struct DownloadOptions {
    /// 仓库 ID，例如 `"meta-llama/Llama-2-7b-hf"`。
    pub repo_id: String,
    /// 分支、tag 或 commit hash。`None` 时使用平台默认分支。
    pub revision: Option<String>,
    /// 本地根目录，库会在其下自动创建 `<owner>--<model>` 子目录。
    pub save_dir: PathBuf,
    /// 允许下载的相对路径白名单，`None` 表示下载全部文件。
    pub files: Option<Vec<String>>,
}

/// 模型下载器，持有 HTTP 客户端与配置，可复用于多次下载。
pub struct ModelDownloader {
    /// reqwest::Client 内部已是 Arc，无需再套一层 Arc。
    client: reqwest::Client,
    provider: HubProvider,
    concurrency: usize,
    max_retries: u32,
}

impl ModelDownloader {
    const DEFAULT_CONCURRENCY: usize = 4;
    const DEFAULT_MAX_RETRIES: u32 = 3;

    /// 为指定平台创建下载器。
    pub fn new(provider: HubProvider) -> Result<Self> {
        let client = Self::build_client(&provider)?;
        Ok(Self {
            client,
            provider,
            concurrency: Self::DEFAULT_CONCURRENCY,
            max_retries: Self::DEFAULT_MAX_RETRIES,
        })
    }

    /// 设置同时进行的最大下载数（默认 4，最小 1）。
    pub fn with_concurrency(mut self, n: usize) -> Self {
        self.concurrency = n.max(1);
        self
    }

    /// 设置每个文件的最大重试次数（默认 3）。
    pub fn with_max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    /// 执行下载。
    pub async fn download(&self, options: DownloadOptions) -> Result<()> {
        Self::validate_options(&options)?;

        let revision = options
            .revision
            .as_deref()
            .unwrap_or_else(|| self.provider.default_revision());

        let model_dir = options
            .repo_id
            .split('/')
            .fold(options.save_dir.clone(), |p, c| p.join(c));
        tokio::fs::create_dir_all(&model_dir).await?;

        let files = match &self.provider {
            HubProvider::HuggingFace { .. } => {
                self.get_hf_files(&options.repo_id, revision).await?
            }
            HubProvider::ModelScope { .. } => self.get_ms_files(&options.repo_id, revision).await?,
        };

        // 使用 HashSet 将过滤从 O(n×m) 降为 O(1)
        let filter: Option<HashSet<String>> = options.files.map(|v| v.into_iter().collect());

        let sem = Arc::new(Semaphore::new(self.concurrency));
        let mut join_set: JoinSet<Result<()>> = JoinSet::new();

        for file in files {
            if let Some(ref set) = filter {
                if !set.contains(&file.path) {
                    continue;
                }
            }

            // 路径穿越校验：在派发任务前提前验证，快速失败
            let dest = safe_join(&model_dir, &file.path)?;
            let client = self.client.clone();
            let sem = Arc::clone(&sem);
            let max_retries = self.max_retries;

            join_set.spawn(async move {
                // 通过信号量限制并发数，_permit 在任务结束时自动释放
                let _permit = sem.acquire().await.expect("信号量意外关闭");
                with_retry(max_retries, || {
                    let c = client.clone();
                    let f = file.clone();
                    let d = dest.clone();
                    async move { download_single_file(c, f, d).await }
                })
                .await
            });
        }

        // 使用 JoinSet 收集结果；任意任务失败时立即 abort 其余任务
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    join_set.abort_all();
                    return Err(e);
                }
                // abort_all 后剩余任务以 Cancelled 结束，忽略即可
                Err(e) if e.is_cancelled() => {}
                Err(e) => {
                    join_set.abort_all();
                    bail!("下载任务发生 panic: {}", e);
                }
            }
        }

        Ok(())
    }

    // ── 私有方法 ───────────────────────────────────────────────────────────────

    fn build_client(provider: &HubProvider) -> Result<reqwest::Client> {
        let mut headers = reqwest::header::HeaderMap::new();

        // 使用语义化 UA，而非伪造浏览器标识
        headers.insert(
            reqwest::header::USER_AGENT,
            concat!("model-hub/", env!("CARGO_PKG_VERSION")).parse()?,
        );

        // 两个平台鉴权逻辑相同，统一处理，消除重复
        if let Some(token) = provider.token() {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", token).parse()?,
            );
        }

        Ok(reqwest::Client::builder()
            .default_headers(headers)
            .build()?)
    }

    fn validate_options(options: &DownloadOptions) -> Result<()> {
        if options.repo_id.is_empty() {
            bail!("repo_id 不能为空");
        }
        if options.repo_id.contains("..") {
            bail!("repo_id 含有非法字符 '..'");
        }
        if let Some(ref files) = options.files {
            for path in files {
                if path.contains("..") || path.starts_with('/') || path.starts_with('\\') {
                    bail!("files 列表中含有非法路径: {:?}", path);
                }
            }
        }
        Ok(())
    }

    /// 获取 HuggingFace 文件列表，支持递归子目录与 Link 分页。
    async fn get_hf_files(&self, repo_id: &str, revision: &str) -> Result<Vec<UnifiedFile>> {
        let base_url =
            std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://huggingface.co".to_string());

        let mut all_files = Vec::new();
        // ?recursive=1 获取所有子目录文件
        let mut next_url: Option<String> = Some(format!(
            "{}/api/models/{}/tree/{}?recursive=1",
            base_url, repo_id, revision
        ));

        while let Some(url) = next_url.take() {
            let resp = self.client.get(&url).send().await?;
            if !resp.status().is_success() {
                bail!("HuggingFace API 请求失败 (HTTP {}): {}", resp.status(), url);
            }

            // 从 Link 响应头提取下一页 URL（分页处理）
            next_url = resp
                .headers()
                .get(reqwest::header::LINK)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_link_next);

            let page: Vec<HfFile> = resp.json().await?;
            all_files.extend(page.into_iter().filter(|f| f.r#type == "file").map(|f| {
                UnifiedFile {
                    download_url: format!(
                        "{}/{}/resolve/{}/{}",
                        base_url, repo_id, revision, f.path
                    ),
                    path: f.path,
                    size: f.size,
                }
            }));
        }

        Ok(all_files)
    }

    /// 获取 ModelScope 文件列表。
    async fn get_ms_files(&self, repo_id: &str, revision: &str) -> Result<Vec<UnifiedFile>> {
        let url = format!(
            "https://modelscope.cn/api/v1/models/{}/repo/files?Recursive=true&Revision={}",
            repo_id, revision
        );

        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            bail!("ModelScope API 请求失败 (HTTP {})", resp.status());
        }

        let parsed: MsResponse = resp.json().await?;
        if !parsed.success {
            bail!("ModelScope API 返回失败状态");
        }

        let files = parsed.data.context("未获取到 ModelScope 文件数据")?.files;

        Ok(files
            .into_iter()
            .filter(|f| f.r#type == "blob")
            .map(|f| UnifiedFile {
                download_url: format!(
                    "https://modelscope.cn/models/{}/resolve/{}/{}",
                    repo_id, revision, f.path
                ),
                path: f.path,
                size: f.size,
            })
            .collect())
    }
}

// ── 辅助函数 ───────────────────────────────────────────────────────────────────

/// 安全路径拼接：过滤 `..` / 绝对路径组件，并在最终验证 dest 在 base 内。
///
/// 防止服务端返回恶意路径（路径穿越攻击）时写入 model_dir 之外的位置。
fn safe_join(base: &Path, file_path: &str) -> Result<PathBuf> {
    let clean: PathBuf = file_path
        .split('/')
        .filter(|c| !c.is_empty() && *c != "." && *c != "..")
        .collect();

    let dest = base.join(&clean);

    // 双重保险：即使 filter 有遗漏，canonicalize 前的前缀检查也能兜底
    if !dest.starts_with(base) {
        bail!("检测到非法路径（路径穿越）: {:?}", file_path);
    }
    Ok(dest)
}

/// 解析 HTTP `Link` 响应头，返回 `rel="next"` 对应的 URL。
///
/// 格式示例：`<https://...?cursor=xxx>; rel="next", <https://...>; rel="last"`
fn parse_link_next(header: &str) -> Option<String> {
    header.split(',').find_map(|part| {
        let mut seg = part.trim().splitn(2, ';');
        let url_part = seg.next()?.trim();
        let rel_part = seg.next()?.trim();
        if rel_part == r#"rel="next""# {
            Some(
                url_part
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .to_string(),
            )
        } else {
            None
        }
    })
}

/// 带指数退避的重试包装器。
///
/// - 首次立即执行，失败后等待 1 → 2 → 4 → … 秒（上限 60 秒）再重试。
/// - `max_retries = 0` 表示不重试，失败直接返回错误。
async fn with_retry<F, Fut>(max_retries: u32, mut f: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    // 前 max_retries 次：失败后等待再重试
    for attempt in 0..max_retries {
        if f().await.is_ok() {
            return Ok(());
        }
        let secs = 2u64.saturating_pow(attempt).min(60);
        tokio::time::sleep(Duration::from_secs(secs)).await;
    }
    // 末次尝试：直接返回结果，编译器可静态证明此处一定返回，无需 unreachable!()
    f().await
        .map_err(|e| e.context(format!("已重试 {} 次后仍失败", max_retries)))
}

/// 下载单个文件，支持断点续传。
///
/// **续传安全性**：仅当服务端真实返回 `206 Partial Content` 时才以追加模式
/// 写入；若服务端忽略 `Range` 头返回 `200`，则截断重写，防止文件静默损坏。
async fn download_single_file(
    client: reqwest::Client,
    file_info: UnifiedFile,
    dest: PathBuf,
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // 检查本地已有文件大小（使用 tokio::fs 避免在 async 中阻塞线程）
    let existing_size = match tokio::fs::metadata(&dest).await {
        Ok(meta) => {
            let size = meta.len();
            if size == file_info.size {
                return Ok(()); // 文件已完整，直接跳过
            }
            size
        }
        Err(_) => 0,
    };

    let should_resume = existing_size > 0 && existing_size < file_info.size;

    let req = if should_resume {
        client
            .get(&file_info.download_url)
            .header("Range", format!("bytes={}-", existing_size))
    } else {
        client.get(&file_info.download_url)
    };

    let resp = req.send().await?;
    let status = resp.status();

    if !status.is_success() {
        bail!("下载失败: {} (HTTP {})", file_info.path, status);
    }

    // 关键：以实际响应状态码决定写入模式，而非以请求意图决定
    let append = should_resume && status == reqwest::StatusCode::PARTIAL_CONTENT;

    let file = if append {
        tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .append(true)
            .open(&dest)
            .await
    } else {
        tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&dest)
            .await
    }
    .with_context(|| format!("无法打开文件: {:?}", dest))?;

    let mut writer = tokio::io::BufWriter::new(file);
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        writer.write_all(&chunk?).await?;
    }
    writer.flush().await?;

    Ok(())
}
