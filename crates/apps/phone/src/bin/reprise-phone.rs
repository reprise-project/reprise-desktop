fn main() {
    reprise_phone::main(std::env::args_os().skip(1).collect());
}
