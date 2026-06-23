use std::env;

fn main() {
    let arg0 = env::args_os().next().expect("argv[0] is present");

    #[cfg(unix)]
    {
        use std::fmt::Write as _;
        use std::os::unix::ffi::OsStrExt;

        let mut hex = String::new();
        for byte in arg0.as_bytes() {
            write!(&mut hex, "{byte:02x}").expect("writing to a String cannot fail");
        }
        println!("ARGV0_HEX:{hex}");
    }

    #[cfg(not(unix))]
    println!("ARGV0:{}", arg0.to_string_lossy());
}
