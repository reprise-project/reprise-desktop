//! Connect to Server connecting and the password box for SMB and SFTP under each look, written to
//! `target/reprisekit-gallery-netconnect@<scale>.png`.

use reprisekit::controls::form;
use reprisekit::controls::netconnect::{self, Login};
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::theme::{Color, FontConfig, Theme};

fn looks() -> Vec<(String, Theme)> {
    let mut out = vec![
        ("Classic".to_string(), Theme::default()),
        (
            "Blue".to_string(),
            interfaces::blue_theme(reprisekit::blue::Variant::ALL[0]),
        ),
    ];
    for mode in [
        reprisekit::glass::Mode::Light,
        reprisekit::glass::Mode::Dark,
    ] {
        let g = reprisekit::glass::Settings {
            mode,
            ..Default::default()
        };
        let t = interfaces::theme_with(Interface::Glass, Default::default(), &g);
        out.push((format!("Glass {mode:?}"), t));
    }
    out
}

fn lines(s: &str) -> Vec<String> {
    s.lines().map(str::to_string).collect()
}

fn forms() -> Vec<(form::Form, (i32, i32))> {
    let mut connect = netconnect::connect_form(
        &lines(
            "Type the address of a shared folder or a server, for example\nsmb://server/share or sftp://user@server/folder.",
        ),
        "smb://nas/public",
        None,
    );
    netconnect::set_busy(&mut connect, true, "Connecting to nas...");
    let smb = lines(
        "Authentication Required\nEnter user and password for share \u{201c}private\u{201d} on \u{201c}nas\u{201d}:",
    );
    let mut retry = netconnect::login_form(
        &Login {
            lines: &smb,
            user: "alice",
            asks_user: true,
            domain: Some(""),
            can_remember: true,
        },
        None,
    );
    retry.set_text(netconnect::STATUS, netconnect::WRONG);
    let ssh = lines("Enter password for \u{201c}bob\u{201d} on \u{201c}box\u{201d}:");
    let sftp = netconnect::login_form(
        &Login {
            lines: &ssh,
            user: "bob",
            asks_user: false,
            domain: None,
            can_remember: false,
        },
        None,
    );
    vec![
        (connect, netconnect::CONNECT_CLIENT),
        (retry, netconnect::login_client(true)),
        (sftp, netconnect::login_client(false)),
    ]
}

pub fn render(s: i32) -> Canvas {
    let looks = looks();
    let forms = forms();
    let label_w = 90;
    let ch = forms.iter().map(|(_, (_, h))| *h).max().unwrap_or(0);
    let w = label_w + forms.iter().map(|(_, (w, _))| w + 10).sum::<i32>();
    let h = looks.len() as i32 * (ch + 10);
    let mut c = super::blurred_stripes(w * s, h * s);
    let head = TextRenderer::load(&FontConfig::default());
    for (i, (name, base)) in looks.iter().enumerate() {
        let y = i as i32 * (ch + 10);
        head.draw(
            &mut c,
            Rect::new(4, y + 4, label_w - 8, 16).scaled(s),
            name,
            true,
            Color::rgb(0xFF, 0xFF, 0xFF),
            Align::Left,
            11 * s as u32,
        );
        let theme = base.scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        let mut x = label_w;
        for (f, (fw, fh)) in &forms {
            let mut d = Canvas::new((fw * s) as u32, (fh * s) as u32);
            d.fill_rect(d.bounds(), p.cs.button_face);
            form::draw(&mut d, &text, &p, f, true, theme.fonts.size_px);
            c.blit_canvas(x * s, y * s, &d);
            x += fw + 10;
        }
    }
    c
}
