fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        // 아이콘 파일 존재 시에만 적용 (placeholder 단계)
        if std::path::Path::new("resources/icon.ico").exists() {
            res.set_icon("resources/icon.ico");
        }
        if std::path::Path::new("resources/app.manifest").exists() {
            res.set_manifest_file("resources/app.manifest");
        }
        res.set("ProductName", "CherishPack Installer");
        res.set("FileDescription", "CherishPack 모드팩 설치 프로그램");
        res.set("LegalCopyright", "Copyright (c) 2026 CherishPack");
        let _ = res.compile();
    }
}
