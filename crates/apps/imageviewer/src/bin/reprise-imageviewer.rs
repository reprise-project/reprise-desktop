fn main() {
    reprise_imageviewer::main(std::env::args_os().skip(1).collect());
}
