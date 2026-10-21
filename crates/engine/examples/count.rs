//! Lists the registered functions: `cargo run -q -p opensheet_engine --example count`.
//! With a language code (`-- de`), also lists the functions that have no
//! localized name in that language.

fn main() {
    let all = opensheet_engine::functions::all();
    println!("{} functions", all.len());
    match std::env::args().nth(1) {
        Some(lang) => {
            let locale = opensheet_engine::Locale::by_id(&lang);
            let same: Vec<&str> = all
                .iter()
                .map(|d| d.name)
                .filter(|n| locale.fn_to_local(n) == *n)
                .collect();
            println!(
                "{} without a name in '{}': {}",
                same.len(),
                locale.id,
                same.join(" ")
            );
        }
        None => {
            for def in all {
                println!("{}\t{:?}", def.name, def.category);
            }
        }
    }
}
