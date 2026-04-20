fn main() {
    cc::Build::new()
        .file("c/unicorn_ctl_shim.c")
        .warnings(true)
        .extra_warnings(false)
        .compile("unicorn_ctl_shim");
}
