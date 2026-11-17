fn main() {
    // Windows 可执行文件内嵌品牌图标与版本信息：
    // 任务栏、资源管理器和快捷方式都从 exe 的资源段取图标（gpui 没有运行时图标接口）。
    #[cfg(windows)]
    {
        // 失败时 compile 会静默返回；图标缺失先查 rc.exe 与 assets/neverwork.rc。
        let _ = embed_resource::compile("assets/neverwork.rc", embed_resource::NONE);
    }
}
