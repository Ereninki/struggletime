fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/struggletime.ico");
        res.set("ProductName", "StruggleTime");
        res.set(
            "FileDescription",
            "StruggleTime — struggle-weighted coding time",
        );
        if let Err(e) = res.compile() {
            println!("cargo:warning=winres failed: {e}");
        }
    }
}
