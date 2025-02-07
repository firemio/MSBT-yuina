use std::fs;
use std::path::Path;

fn copy_config_file() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let profile = std::env::var("PROFILE").unwrap();
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    
    // ソースの設定ファイル
    let config_src = Path::new(&manifest_dir).join("MSBT-yuina.toml");
    
    // ターゲットディレクトリのパス
    let target_dir = Path::new(&out_dir)
        .ancestors()
        .find(|p| p.ends_with("target"))
        .unwrap()
        .to_path_buf();
    
    // 設定ファイルのコピー先
    let config_dest = target_dir.join(&profile).join("MSBT-yuina.toml");
    
    // 設定ファイルが存在しない場合のみコピー
    if !config_dest.exists() {
        println!("cargo:warning=Copying config file to: {}", config_dest.display());
        fs::create_dir_all(config_dest.parent().unwrap()).unwrap();
        fs::copy(&config_src, &config_dest).unwrap();
    }
}

fn main() {
    // 設定ファイルをコピー
    copy_config_file();
    
    // Windowsリソースのコンパイル
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        embed_resource::compile("src/app.rc", &["" as &str]);
    }
}
