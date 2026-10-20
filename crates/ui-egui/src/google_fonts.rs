//! Type › Browse Google Fonts…: search the open-licence families of the google/fonts repository
//! (`text.googleFonts`) and add them to VectorCraft's Fonts folder (`text.installGoogleFont`).
//! Featured first: Adobe's Source fonts and Noto CJK, and the fonts with the metrics of the PDF
//! standard fonts.

use egui::Ui;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets;

pub const KIND: &str = "googleFonts";

/// Open the dialog.
pub fn open(app: &mut VectorcraftApp) {
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "query": "" })));
}

pub fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    app.ui.dialog = None;
    Ok(Value::Null)
}

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.dialog.clone() else { return };
    let t = Tokens::get(ctx);
    let query = d.str("query");
    let r = app
        .session
        .execute("text.googleFonts", &json!({ "query": query, "limit": 200 }))
        .unwrap_or_else(|e| json!({ "state": "failed", "error": e.to_string() }));
    let state = r["state"].as_str().unwrap_or_default().to_string();
    let downloading = |id: &str| r["installs"].as_array().into_iter().flatten().any(|i| i["id"] == id && i["state"] == "downloading");
    let busy = state == "loading" || r["installs"].as_array().into_iter().flatten().any(|i| i["state"] == "downloading");
    let mut close = false;
    let mut add: Option<String> = None;
    let mut refresh = false;
    crate::dialogs::modal::show(ctx, tl!("Google Fonts"), egui::Id::new("dialog-google-fonts"), -40.0, 22, |ui: &mut Ui| {
        ui.set_width(440.0);
        crate::dialogs::modal::heading(ui, tl!("Google Fonts"));
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(tl!("Open-licence fonts from the google/fonts repository, added to VectorCraft's Fonts folder."))
                .size(11.0)
                .color(t.text_dim),
        );
        ui.add_space(6.0);
        let mut q = query.clone();
        if ui.add(egui::TextEdit::singleline(&mut q).hint_text(tl!("Search")).desired_width(f32::INFINITY)).changed() {
            d.fields.insert("query".into(), json!(q));
        }
        ui.add_space(6.0);
        egui::Frame::NONE.fill(t.input).stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.set_min_height(260.0);
            ui.set_width(ui.available_width());
            match state.as_str() {
                "unavailable" | "failed" => {
                    ui.label(egui::RichText::new(r["error"].as_str().unwrap_or_default()).color(t.text_dim));
                }
                _ if r["families"].as_array().is_none_or(Vec::is_empty) && state == "loading" => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tl!("Downloading the catalog…"));
                    });
                }
                _ => {
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        for f in r["families"].as_array().into_iter().flatten() {
                            let id = f["id"].as_str().unwrap_or_default();
                            ui.horizontal(|ui| {
                                let star = if f["featured"] == true { "★ " } else { "" };
                                ui.label(format!("{star}{}", f["name"].as_str().unwrap_or_default()));
                                let n = f["fonts"].as_u64().unwrap_or(0);
                                ui.label(
                                    egui::RichText::new(format!("{} · {n}", f["licence"].as_str().unwrap_or_default().to_uppercase()))
                                        .size(10.0)
                                        .color(t.text_dim),
                                );
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if downloading(id) {
                                        ui.spinner();
                                    } else if f["installed"] == true {
                                        ui.label(egui::RichText::new(tl!("Added")).color(t.text_dim));
                                    } else if ui.small_button(tl!("Add")).clicked() {
                                        add = Some(id.to_string());
                                    }
                                });
                            });
                        }
                    });
                }
            }
        });
        if let Some(failed) = r["installs"].as_array().into_iter().flatten().find(|i| i["state"] == "failed") {
            ui.label(egui::RichText::new(failed["error"].as_str().unwrap_or_default()).size(11.0).color(egui::Color32::from_rgb(230, 90, 90)));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let total = r["total"].as_u64().unwrap_or(0);
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!("{count} families"), &[("count", &total.to_string())])).size(11.0).color(t.text_dim));
            if ui.add_enabled(state != "loading" && state != "unavailable", egui::Button::new(tl!("Refresh"))).clicked() {
                refresh = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::primary_button(ui, tl!("Done")).clicked() {
                    close = true;
                }
            });
        });
    });
    if let Some(id) = add
        && let Err(e) = app.run("text.installGoogleFont", json!({ "id": id }))
    {
        app.ui.status = e;
    }
    if refresh {
        // The answer is the catalog's state, read again next frame.
        let _ = app.session.execute("text.googleFonts", &json!({ "refresh": true }));
    }
    if busy {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.dialog = None;
    } else if app.ui.dialog.is_some() {
        app.ui.dialog = Some(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without a downloader (tests, the web) the dialog says so instead of listing fonts.
    #[test]
    fn without_a_downloader_the_dialog_says_so() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("ui.googleFontsDialog", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| show(app, ui.ctx()));
        assert!(text.contains("Google Fonts") && text.contains("desktop app"), "{text}");
        crate::dialogs::cancel(&mut app);
    }
}
