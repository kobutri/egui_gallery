use std::{borrow::Cow, sync::Arc};

use eframe::App;
use egui::epaint::RectShape;
use egui::load::TexturePoll;
use egui::{Color32, CornerRadius, Image, Pos2, Sense, Vec2, Widget, mutex::RwLock};
use egui_taffy::{
    TuiBuilderLogic, tid, tui,
    virtual_tui::{VirtualGridRowHelper, VirtualGridRowHelperParams},
};
use shared::ImageResponse;
use taffy::{
    prelude::{auto, flex, length, percent, span},
    style_helpers,
};

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_resizable(true),
        ..Default::default()
    };
    eframe::run_native(
        "Gallery",
        native_options,
        Box::new(|cc| Ok(Box::new(Gallery::new(cc)))),
    )?;
    Ok(())
}

struct State {
    page: usize,
    limit: usize,
    images: Vec<ImageResponse>,
    loading: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            page: Default::default(),
            limit: 10,
            images: Default::default(),
            loading: Default::default(),
        }
    }
}

impl State {
    fn try_fetch(state: Arc<RwLock<Self>>, rt: &tokio::runtime::Runtime, ctx: egui::Context) {
        rt.spawn(async move {
            {
                let mut state = state.write();
                if state.loading {
                    return;
                }
                state.loading = true;
            }
            let State { limit, page, .. } = *state.read();
            let images = match reqwest::get(format!(
                "http://localhost:3000/images/fetch?page={page}&limit={limit}"
            ))
            .await
            {
                Ok(response) => match response.json::<Vec<ImageResponse>>().await {
                    Ok(imgages) => imgages,
                    Err(e) => {
                        eprintln!("{e}");
                        state.write().loading = false;
                        return;
                    }
                },
                Err(e) => {
                    eprintln!("{e}");
                    state.write().loading = false;
                    return;
                }
            };
            println!("fteched {} images", images.len());
            {
                let mut state = state.write();
                state.images.extend(images);
                state.page += 1;
                state.loading = false;
            }
            ctx.request_repaint();
        });
    }
}

struct Gallery {
    rt: tokio::runtime::Runtime,
    state: Arc<RwLock<State>>,
}

impl Gallery {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        egui_thumbhash::register(&cc.egui_ctx);
        cc.egui_ctx.all_styles_mut(|style| {
            style.wrap_mode = Some(egui::TextWrapMode::Extend);
        });
        Gallery {
            rt: tokio::runtime::Runtime::new().unwrap(),
            state: Arc::new(RwLock::new(State::default())),
        }
    }
}

struct CoverImage<'a> {
    source: Cow<'a, str>,
    size: Vec2,
}

impl<'a> CoverImage<'a> {
    fn new(source: impl Into<Cow<'a, str>>, size: Vec2) -> Self {
        Self {
            source: source.into(),
            size,
        }
    }
}

impl Widget for CoverImage<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let mut size = self.size;
        if !size.x.is_finite() {
            size.x = ui.available_width();
        }
        if !size.y.is_finite() {
            size.y = ui.available_height();
        }
        size.x = size.x.max(1.0);
        size.y = size.y.max(1.0);
        let rounding = CornerRadius::ZERO;

        let image = Image::new(self.source.as_ref()).maintain_aspect_ratio(true);
        let load_result = image.load_for_size(ui.ctx(), size);

        let (rect, response) = ui.allocate_exact_size(size, Sense::click());

        if !ui.is_rect_visible(rect) {
            return response;
        }

        match load_result {
            Ok(TexturePoll::Ready { texture, .. }) => {
                let tex_size = Vec2::new(texture.size[0] as f32, texture.size[1] as f32);
                if tex_size.x > 0.0 && tex_size.y > 0.0 {
                    let mut uv_min = Pos2::new(0.0, 0.0);
                    let mut uv_max = Pos2::new(1.0, 1.0);

                    let tex_ratio = tex_size.x / tex_size.y;
                    let target_ratio = size.x / size.y;

                    if target_ratio > tex_ratio {
                        // Target is wider -> crop vertically
                        let scale = target_ratio / tex_ratio;
                        let visible = 1.0 / scale;
                        let offset = (1.0 - visible) / 2.0;
                        uv_min.y = offset;
                        uv_max.y = 1.0 - offset;
                    } else {
                        // Target is taller -> crop horizontally
                        let scale = tex_ratio / target_ratio;
                        let visible = 1.0 / scale;
                        let offset = (1.0 - visible) / 2.0;
                        uv_min.x = offset;
                        uv_max.x = 1.0 - offset;
                    }

                    let shape = RectShape::filled(rect, rounding, Color32::WHITE)
                        .with_texture(texture.id, egui::Rect::from_min_max(uv_min, uv_max));
                    ui.painter().add(shape);
                } else {
                    ui.painter()
                        .rect_filled(rect, rounding, Color32::from_gray(30));
                }
            }
            _ => {
                ui.painter()
                    .rect_filled(rect, rounding, Color32::from_gray(30));
            }
        }

        response
    }
}

impl App for Gallery {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let state = self.state.read();
        if state.images.is_empty() {
            State::try_fetch(self.state.clone(), &self.rt, ctx.clone());
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Gallery");
            tui(ui, ui.id().with("virtual_grid"))
                .reserve_available_space()
                .style(taffy::Style {
                    flex_direction: taffy::FlexDirection::Column,
                    size: percent(1.),
                    max_size: percent(1.),
                    ..Default::default()
                })
                .show(|tui| {
                    tui.style(taffy::Style {
                        display: taffy::Display::Grid,
                        overflow: taffy::Point {
                            x: taffy::Overflow::Visible,
                            y: taffy::Overflow::Scroll,
                        },
                        grid_template_columns: vec![flex(1.0), flex(1.0)],
                        size: taffy::Size {
                            width: percent(100.),
                            height: auto(),
                        },
                        max_size: percent(1.),
                        grid_auto_rows: vec![length(300.)],
                        ..Default::default()
                    })
                    .add(|tui| {
                        let row_count = state.images.len() / 2;
                        VirtualGridRowHelper::show(
                            VirtualGridRowHelperParams {
                                header_row_count: 1,
                                row_count: row_count,
                            },
                            tui,
                            |tui, info| {
                                let mut idgen = info.id_gen();
                                let mut_grid_row_param = info.grid_row_setter();

                                for cidx in 0..2 {
                                    let index = info.idx * 2 + cidx;
                                    if index >= state.images.len() - 1 {
                                        State::try_fetch(self.state.clone(), &self.rt, ctx.clone());
                                    }
                                    if index >= state.images.len() {
                                        continue;
                                    }
                                    let image_response = &state.images[index];
                                    tui.id(idgen())
                                        .mut_style(&mut_grid_row_param)
                                        .mut_style(|style| {
                                            style.padding = length(2.);
                                            style.size.width = percent(100.0);
                                            style.size.height = length(300.0);
                                        })
                                        .ui(|ui| {
                                            let mut size = ui.available_size();
                                            if !size.y.is_finite() || size.y <= 0.0 {
                                                size.y = 300.0;
                                            } else {
                                                size.y = 300.0;
                                            }
                                            let url = format!(
                                                "http://localhost:3000/{}",
                                                image_response.path
                                            );
                                            ui.add(CoverImage::new(url, size));
                                        });
                                }
                            },
                        );

                        tui.sticky([false, true].into())
                            .style(taffy::Style {
                                flex_direction: taffy::FlexDirection::Column,
                                grid_row: style_helpers::line(1),
                                padding: length(4.),
                                align_items: Some(taffy::AlignItems::Center),
                                grid_column: span(2),
                                ..Default::default()
                            })
                            .id(tid(("header", 1)))
                            .add_with_background_color(|tui| tui.label("Colpan 2 header"));
                    });
                });
        });
    }
}
