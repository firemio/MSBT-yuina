//! GitHub Releases を使った自動更新。
//!
//! 流れ:
//! 1. `spawn_check` — 最新リリースをバックグラウンドで確認（起動時／Helpメニューから）
//! 2. `spawn_download_and_install` — 新しい exe をダウンロードし、実行中の exe を
//!    その場で差し替える（`self-replace` が Windows のリネーム手順を面倒みてくれる）
//! 3. 再起動すると新バージョンが起動する
//!
//! ネットワークエラー等はすべて `UpdateStatus::Failed` に落とし、アプリ本体は止めない。

use log::{error, info};
use serde::Deserialize;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const REPO_OWNER: &str = "firemio";
pub const REPO_NAME: &str = "MSBT-yuina";
/// 自動更新で探すリリースアセット名（既存リリースと同じ素の exe）
pub const ASSET_NAME: &str = "MSBT-yuina.exe";
/// このビルドのバージョン
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateStatus {
    Idle,
    Checking,
    UpToDate,
    Available { version: String, url: String },
    Downloading { version: String },
    /// exe の差し替えが完了。再起動すると新バージョンが有効になる
    Ready { version: String },
    Failed(String),
}

pub type SharedStatus = Arc<Mutex<UpdateStatus>>;

pub fn new_shared_status() -> SharedStatus {
    Arc::new(Mutex::new(UpdateStatus::Idle))
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// "v1.2.3" などを数値列にする（先頭の v は無視、数値でない部分以降は捨てる）
fn parse_version(s: &str) -> Vec<u64> {
    s.trim()
        .trim_start_matches(['v', 'V'])
        .split(['.', '-'])
        .map_while(|p| p.parse::<u64>().ok())
        .collect()
}

/// candidate が current より新しいか。欠けた桁は 0 として比較する（"1.1" == "1.1.0"）
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let mut a = parse_version(candidate);
    let mut b = parse_version(current);
    if a.is_empty() {
        return false; // タグが解釈できない場合は更新を提案しない
    }
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

fn user_agent() -> String {
    format!("MSBT-yuina/{CURRENT_VERSION}")
}

fn fetch_latest_release() -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{REPO_OWNER}/{REPO_NAME}/releases/latest");
    let resp = ureq::get(&url)
        .set("User-Agent", &user_agent())
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(15))
        .call()
        .map_err(|e| format!("更新情報の取得に失敗しました: {e}"))?;
    resp.into_json::<Release>()
        .map_err(|e| format!("更新情報の解析に失敗しました: {e}"))
}

/// 最新リリースの確認をバックグラウンドで開始する。
/// すでに確認中／ダウンロード中／適用済みの場合は何もしない。
pub fn spawn_check(status: SharedStatus, ctx: egui::Context) {
    {
        let mut s = status.lock().unwrap();
        match *s {
            UpdateStatus::Checking | UpdateStatus::Downloading { .. } | UpdateStatus::Ready { .. } => {
                return
            }
            _ => *s = UpdateStatus::Checking,
        }
    }
    info!("更新確認を開始します（現在 v{CURRENT_VERSION}）");
    std::thread::spawn(move || {
        let result = (|| -> Result<UpdateStatus, String> {
            let release = fetch_latest_release()?;
            let latest = release.tag_name.clone();
            info!("最新リリース: {latest}（現在 v{CURRENT_VERSION}）");
            if !is_newer(&latest, CURRENT_VERSION) {
                return Ok(UpdateStatus::UpToDate);
            }
            let asset = release
                .assets
                .iter()
                .find(|a| a.name.eq_ignore_ascii_case(ASSET_NAME))
                .ok_or_else(|| format!("リリース {latest} に {ASSET_NAME} が見つかりません"))?;
            Ok(UpdateStatus::Available {
                version: latest,
                url: asset.browser_download_url.clone(),
            })
        })();
        let mut s = status.lock().unwrap();
        *s = match result {
            Ok(st) => st,
            Err(e) => {
                error!("更新確認: {e}");
                UpdateStatus::Failed(e)
            }
        };
        drop(s);
        ctx.request_repaint();
    });
}

/// 新バージョンのダウンロードと exe 差し替えをバックグラウンドで開始する。
/// `exe_path` は起動時に取得した自分自身のパス（差し替え後の current_exe() は
/// リネーム後のパスを返し得るため、起動時の値を使う）。
pub fn spawn_download_and_install(
    status: SharedStatus,
    version: String,
    url: String,
    exe_path: PathBuf,
    ctx: egui::Context,
) {
    {
        let mut s = status.lock().unwrap();
        if matches!(*s, UpdateStatus::Downloading { .. } | UpdateStatus::Ready { .. }) {
            return;
        }
        *s = UpdateStatus::Downloading { version: version.clone() };
    }
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            info!("更新のダウンロード開始: {url}");
            let resp = ureq::get(&url)
                .set("User-Agent", &user_agent())
                .timeout(Duration::from_secs(600))
                .call()
                .map_err(|e| format!("ダウンロードに失敗しました: {e}"))?;

            let mut data = Vec::new();
            resp.into_reader()
                .take(200 * 1024 * 1024) // 念のため 200MB 上限
                .read_to_end(&mut data)
                .map_err(|e| format!("ダウンロードに失敗しました: {e}"))?;

            // 妥当性チェック: PE 実行ファイル（MZ ヘッダ）かつ十分なサイズであること
            if data.len() < 1024 * 1024 || !data.starts_with(b"MZ") {
                return Err("ダウンロードしたファイルが実行ファイルではありません".to_string());
            }

            let tmp = exe_path.with_extension("update.tmp");
            std::fs::write(&tmp, &data).map_err(|e| {
                format!(
                    "一時ファイルを書き込めません（書き込み権限のない場所から実行している可能性）: {e}"
                )
            })?;

            let replace_result = self_replace::self_replace(&tmp)
                .map_err(|e| format!("実行ファイルの差し替えに失敗しました: {e}"));
            let _ = std::fs::remove_file(&tmp);
            replace_result?;
            info!("更新 {version} を適用しました（再起動で有効）");
            Ok(())
        })();
        let mut s = status.lock().unwrap();
        *s = match result {
            Ok(()) => UpdateStatus::Ready { version },
            Err(e) => {
                error!("更新適用: {e}");
                UpdateStatus::Failed(e)
            }
        };
        drop(s);
        ctx.request_repaint();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 実ネットワークを使う確認用テスト（cargo test -- --ignored で実行）
    #[test]
    #[ignore]
    fn live_fetch_latest_release() {
        let rel = fetch_latest_release().expect("GitHub API から取得できない");
        println!("latest tag: {}", rel.tag_name);
        assert!(!rel.tag_name.is_empty());
        assert!(
            rel.assets.iter().any(|a| a.name.eq_ignore_ascii_case(ASSET_NAME)),
            "アセット {ASSET_NAME} が無い: {:?}",
            rel.assets.iter().map(|a| &a.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn version_parsing() {
        assert_eq!(parse_version("v1.2.3"), vec![1, 2, 3]);
        assert_eq!(parse_version("1.10"), vec![1, 10]);
        assert_eq!(parse_version("V2.0.1-beta"), vec![2, 0, 1]); // beta は数値でないので打ち切り
        assert_eq!(parse_version("release"), Vec::<u64>::new());
    }

    #[test]
    fn newer_comparison() {
        assert!(is_newer("v1.1.0", "1.0.4"));
        assert!(is_newer("v1.0.10", "1.0.9"));
        assert!(!is_newer("v1.0.4", "1.0.4"));
        assert!(!is_newer("v1.0.3", "1.0.4"));
        assert!(!is_newer("1.1", "1.1.0")); // 同値
        assert!(is_newer("1.1.1", "1.1"));
        assert!(!is_newer("garbage", "1.0.0")); // 解釈できないタグでは更新しない
        assert!(is_newer("2.0", "1.9.9"));
    }
}
