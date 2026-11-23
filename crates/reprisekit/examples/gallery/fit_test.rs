use super::{collect_pages, write_pages, write_scale};
use reprisekit::pins;
use std::path::Path;

/// Every page at scale 1 and 2, drawn with Tahoma and as on a machine without it, lists each
/// line DejaVu Sans cuts or ellipsizes more often than Tahoma against
/// `tests/pins/text_fit_without_tahoma.txt`.
#[test]
fn every_page_without_tahoma_fits_the_text_it_fits_with_tahoma() {
    let found = pins::unfit_without_tahoma(|| {
        collect_pages(Some(Box::new(|_, page| {
            let page = page.trim_start_matches("reprisekit-gallery-");
            pins::surface_drawn(page.trim_end_matches(".png"));
        })));
        write_pages(true, &[None]);
        write_scale(2);
        collect_pages(None);
    });
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pins/text_fit_without_tahoma.txt");
    pins::check_list(&path, &found, "lines DejaVu Sans cuts and Tahoma does not");
}
