use std::sync::Arc;

use eframe::App;
use egui::{Image, Pos2, Vec2, mutex::RwLock};
use egui::{OpenUrl, Rect, Sense};
use egui_taffy::{
    TuiBuilderLogic, tid, tui,
    virtual_tui::{VirtualGridRowHelper, VirtualGridRowHelperParams},
};
use egui_thumbhash::ThumbhashImage;
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
                    if state.images.is_empty() && state.loading {
                        tui.ui(|ui| ui.spinner());
                        return;
                    }
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
                        grid_auto_rows: vec![auto()],
                        ..Default::default()
                    })
                    .add(|tui| {
                        let row_count = state.images.len() / 2;
                        VirtualGridRowHelper::show(
                            VirtualGridRowHelperParams {
                                header_row_count: 1,
                                row_count,
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
                                            style.aspect_ratio = Some(1.0);
                                        })
                                        .ui(|ui| {
                                            let url = format!(
                                                "http://localhost:3000/{}",
                                                image_response.path
                                            );

                                            let image = Image::new(&url)
                                                .shrink_to_fit()
                                                .maintain_aspect_ratio(false)
                                                .uv(calc_cover_uv(
                                                    ui.available_size(),
                                                    Vec2::new(
                                                        image_response.width as f32,
                                                        image_response.height as f32,
                                                    ),
                                                ))
                                                .sense(Sense::click());

                                            if ThumbhashImage::new(image, &image_response.hash)
                                                .ui(ui)
                                                .clicked()
                                            {
                                                ctx.open_url(OpenUrl { url, new_tab: true });
                                            }
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

fn calc_cover_uv(available_size: Vec2, image_source_size: Vec2) -> Rect {
    let mut min = Pos2::ZERO;
    let mut max = Pos2::new(1.0, 1.0);

    let image_source_aspect = image_source_size.x / image_source_size.y;
    let available_aspect = available_size.x / available_size.y;

    if available_aspect >= image_source_aspect {
        let height_ratio = image_source_aspect / available_aspect;
        min.y = (1.0 - height_ratio) / 2.0;
        max.y = height_ratio / 2.0 + 0.5;
    } else {
        let width_ratio = available_aspect / image_source_aspect;
        min.x = (1.0 - width_ratio) / 2.0;
        max.x = width_ratio / 2.0 + 0.5;
    }

    Rect { min, max }
}
