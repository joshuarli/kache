extern "C" {
    fn answer() -> i32;
    fn adjust() -> i32;
}

fn main() {
    println!("{}", unsafe { answer() + adjust() });
}
