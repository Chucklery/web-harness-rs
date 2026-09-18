pub fn status() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macOS target detected; Seatbelt enforcement not enabled yet (exec requires approval)"
    }
    #[cfg(not(target_os = "macos"))]
    {
        "no OS sandbox backend enabled (exec requires approval)"
    }
}
