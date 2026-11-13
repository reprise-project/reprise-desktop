use super::*;
use crate::proxyhost::Proxy;
use reprisefs::proxy;
use std::cell::RefCell;

thread_local! {
    static PUBLISHED: RefCell<Vec<Vec<String>>> = const { RefCell::new(Vec::new()) };
}

fn record(args: &[Vec<String>]) -> Result<(), String> {
    PUBLISHED.with_borrow_mut(|p| p.extend(args.iter().cloned()));
    Ok(())
}

fn published() -> Vec<Vec<String>> {
    PUBLISHED.with_borrow_mut(std::mem::take)
}

fn change(a: &mut SettingsApp, id: &'static str) {
    a.sheet_event(propsheet::Event::Page(
        Page::Network.index(),
        form::Event::Changed(id),
    ));
}

fn type_in(a: &mut SettingsApp, id: &'static str, text: &str) {
    let page = &mut a.sheet.pages[Page::Network.index()];
    page.show_edit_text(id, text);
    change(a, id);
}

fn set_port(a: &mut SettingsApp, i: usize, port: i32) {
    let page = &mut a.sheet.pages[Page::Network.index()];
    if let Some(form::Control::UpDown(u)) =
        page.item_mut(pages::PROXY_PORTS[i]).map(|i| &mut i.control)
    {
        u.set(port);
    }
    change(a, pages::PROXY_PORTS[i]);
}

fn pick(a: &mut SettingsApp, id: &'static str) {
    let page = &mut a.sheet.pages[Page::Network.index()];
    for r in [pages::PROXY_NONE, pages::PROXY_MANUAL, pages::PROXY_AUTO] {
        page.set_check(r, r == id);
    }
    change(a, id);
}

#[test]
fn apply_writes_the_file_and_the_manager_and_no_proxy_takes_both_back() {
    let dir = tmp("proxy");
    let file = dir.join("environment.d").join(proxy::FILE);
    let mut a = app("proxy-app");
    a.take_proxy(Proxy::open(file.clone(), record));
    assert!(!a.dirty());

    pick(&mut a, pages::PROXY_MANUAL);
    type_in(&mut a, pages::PROXY_HOSTS[0], "proxy.lan");
    set_port(&mut a, 0, 3128);
    assert!(a.dirty());
    assert!(a.apply());
    assert!(!a.dirty());
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "# Reprise Settings > Network writes this file.\n\
         http_proxy=http://proxy.lan:3128\nHTTP_PROXY=http://proxy.lan:3128\n\
         no_proxy=localhost,127.0.0.1,::1\nNO_PROXY=localhost,127.0.0.1,::1\n"
    );
    assert_eq!(
        published()[0][..2],
        ["set-environment", "http_proxy=http://proxy.lan:3128"]
    );

    pick(&mut a, pages::PROXY_NONE);
    assert!(a.apply());
    assert!(!file.exists(), "No proxy removes the file");
    assert_eq!(
        published(),
        [[
            "unset-environment",
            "http_proxy",
            "HTTP_PROXY",
            "no_proxy",
            "NO_PROXY"
        ]
        .map(String::from)
        .to_vec()]
    );
}

#[test]
fn a_wrong_field_fails_apply_and_writes_nothing() {
    let dir = tmp("proxy-wrong");
    let file = dir.join(proxy::FILE);
    let mut a = app("proxy-wrong-app");
    a.take_proxy(Proxy::open(file.clone(), record));
    pick(&mut a, pages::PROXY_AUTO);
    assert!(!a.apply());
    assert!(a.error.is_some());
    assert!(a.dirty(), "the sheet keeps the change");
    assert!(!file.exists());
    assert!(published().is_empty());
}

#[test]
fn the_page_shows_the_file_and_greys_the_modes_not_picked() {
    let dir = tmp("proxy-show");
    let file = dir.join(proxy::FILE);
    std::fs::write(&file, "auto_proxy=http://wpad.lan/wpad.dat\n").unwrap();
    let mut a = app("proxy-show-app");
    a.take_proxy(Proxy::open(file, record));
    let page = &a.sheet.pages[Page::Network.index()];
    assert_eq!(page.checked(pages::PROXY_AUTO), Some(true));
    assert_eq!(page.edit_text(pages::PROXY_PAC), "http://wpad.lan/wpad.dat");
    let lit = |a: &SettingsApp, id| match &a.sheet.pages[Page::Network.index()]
        .item(id)
        .unwrap()
        .control
    {
        form::Control::Edit { enabled, .. } => *enabled,
        c => panic!("{id} is an edit, not {c:?}"),
    };
    assert!(lit(&a, pages::PROXY_PAC));
    assert!(!lit(&a, pages::PROXY_HOSTS[0]));
    pick(&mut a, pages::PROXY_MANUAL);
    assert!(!lit(&a, pages::PROXY_PAC));
    assert!(lit(&a, pages::PROXY_IGNORE));
    a.revert();
    assert!(lit(&a, pages::PROXY_PAC), "Cancel shows the file again");
    assert!(!a.dirty());
}
