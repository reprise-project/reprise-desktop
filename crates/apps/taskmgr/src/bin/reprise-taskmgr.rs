fn main() {
    reprise_taskmgr::main(std::env::args_os().skip(1).collect());
}
