//! Ручная проверка сети: cargo run -p noveltl-core --example check_net -- [режим] [bind_ip]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("system");
    let bind = args.get(2).map(String::as_str).unwrap_or("");
    match noveltl_core::net::make_http(mode, "", bind, "", "") {
        Ok(http) => println!("{mode} {bind}: {:?}", noveltl_core::net::check_google(&http)),
        Err(e) => println!("ошибка настройки: {e}"),
    }
}
