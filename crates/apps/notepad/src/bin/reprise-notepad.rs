fn main() {
    reprise_notepad::main(std::env::args_os().skip(1).collect());
}
