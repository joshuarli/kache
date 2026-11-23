fn main() {
    println!("cargo:rerun-if-changed=native/answer.h");
    println!("cargo:rerun-if-changed=native/answer.c");
    println!("cargo:rerun-if-changed=native/adjust.cpp");
    cc::Build::new().file("native/answer.c").compile("answer");
    cc::Build::new().cpp(true).file("native/adjust.cpp").compile("adjust");
}
