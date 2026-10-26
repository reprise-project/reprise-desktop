fn main() {
    reprise_drives::main(std::env::args_os().skip(1).collect());
}
