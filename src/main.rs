use eframe::egui::{self, Color32, Pos2, Rect, Sense, Vec2};
use std::path::{Path, PathBuf};

struct ImageViewer {
    current_image: Option<egui::TextureHandle>,
    current_path: Option<PathBuf>,
    scale: f32,
    offset: Vec2,
    dragging: bool,
    drag_start: Option<Pos2>,
    last_cursor_pos: Option<Pos2>,
    image_size: Option<[u32; 2]>,
    should_load_next: bool,
    should_load_prev: bool,
    open_file_dialog: bool,
    image_list: Vec<PathBuf>,
    current_image_index: Option<usize>,
    available_size: Option<Vec2>,
    initial_path: Option<PathBuf>,
}

impl Default for ImageViewer {
    fn default() -> Self {
        Self {
            current_image: None,
            current_path: None,
            scale: 1.0,
            offset: Vec2::ZERO,
            dragging: false,
            drag_start: None,
            last_cursor_pos: None,
            image_size: None,
            should_load_next: false,
            should_load_prev: false,
            open_file_dialog: false,
            image_list: Vec::new(),
            current_image_index: None,
            available_size: None,
            initial_path: std::env::args().nth(1).map(PathBuf::from),
        }
    }
}

impl ImageViewer {
    fn update_image_list(&mut self, current_path: &Path) {
        if let Some(parent) = current_path.parent() {
            let mut files: Vec<_> = std::fs::read_dir(parent)
                .unwrap()
                .filter_map(|entry| {
                    let entry = entry.ok()?;
                    let path = entry.path();
                    if path.extension().map_or(false, |ext| {
                        let ext = ext.to_string_lossy().to_lowercase();
                        ["jpg", "jpeg", "png", "gif", "webp", "bmp"].contains(&ext.as_str())
                    }) {
                        Some(path)
                    } else {
                        None
                    }
                })
                .collect();
            
            files.sort();
            self.image_list = files;
            self.current_image_index = self.image_list.iter().position(|p| p == current_path);
        }
    }

    fn update_scale(&mut self, ctx: &egui::Context) {
        if let (Some(available_size), Some(image_size)) = (self.available_size, self.image_size) {
            let image_size = Vec2::new(image_size[0] as f32, image_size[1] as f32);
            let scale_x = available_size.x / image_size.x;
            let scale_y = available_size.y / image_size.y;
            self.scale = scale_x.min(scale_y);
            self.offset = Vec2::ZERO;
            self.update_window_title(ctx);
        }
    }

    fn load_image(&mut self, path: &Path, ctx: &egui::Context) {
        if let Ok(img) = image::open(path) {
            let size = [img.width(), img.height()];
            let image_buffer = img.to_rgba8();
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [size[0] as _, size[1] as _],
                &image_buffer,
            );
            self.current_image = Some(ctx.load_texture(
                "current-image",
                image,
                egui::TextureOptions::default(),
            ));
            self.current_path = Some(path.to_path_buf());
            self.image_size = Some(size);
            self.update_image_list(path);
            self.update_scale(ctx);
            self.update_window_title(ctx);
        }
    }

    fn load_adjacent_image(&mut self, ctx: &egui::Context, next: bool) {
        if let Some(index) = self.current_image_index {
            let new_index = if next {
                (index + 1) % self.image_list.len()
            } else {
                (index + self.image_list.len() - 1) % self.image_list.len()
            };
            let path = self.image_list[new_index].clone();
            self.load_image(&path, ctx);
        }
    }

    fn update_window_title(&self, ctx: &egui::Context) {
        if let Some(path) = &self.current_path {
            let title = format!(
                "Simple Image Viewer - {}",
                path.display(),
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    fn handle_input(&mut self, ui: &egui::Ui) {
        if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            self.should_load_prev = true;
        }
        if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            self.should_load_next = true;
        }
        if ui.input(|i| i.key_pressed(egui::Key::PlusEquals)) {
            self.scale *= 1.1;
            self.scale = self.scale.min(10.0);
            self.update_window_title(ui.ctx());
        }
        if ui.input(|i| i.key_pressed(egui::Key::Minus)) {
            self.scale *= 0.9;
            self.scale = self.scale.max(0.01);
            self.update_window_title(ui.ctx());
        }
    }

    fn draw_checker_background(&self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let painter = ui.painter();
        let checker_size = 10.0;
        
        for i in 0..(rect.width() / checker_size) as i32 {
            for j in 0..(rect.height() / checker_size) as i32 {
                let is_dark = (i + j) % 2 == 0;
                let color = if is_dark {
                    Color32::from_gray(200)
                } else {
                    Color32::from_gray(255)
                };
                
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(
                            rect.min.x + i as f32 * checker_size,
                            rect.min.y + j as f32 * checker_size,
                        ),
                        Vec2::new(checker_size, checker_size),
                    ),
                    0.0,
                    color,
                );
            }
        }
    }
}

impl eframe::App for ImageViewer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Load initial image if provided
        if let Some(path) = self.initial_path.take() {
            if path.exists() {
                self.load_image(&path, ctx);
            }
        }

        if self.should_load_prev {
            self.should_load_prev = false;
            self.load_adjacent_image(ctx, false);
        }
        if self.should_load_next {
            self.should_load_next = false;
            self.load_adjacent_image(ctx, true);
        }

        if self.open_file_dialog {
            self.open_file_dialog = false;
            if let Some(file) = rfd::FileDialog::new()
                .add_filter("Images", &["jpg", "jpeg", "png", "gif", "webp", "bmp"])
                .pick_file()
            {
                self.load_image(&file, ctx);
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let rect = ui.max_rect();
            let new_available_size = ui.available_size();
            
            // Update scale if window size changed
            if self.available_size != Some(new_available_size) {
                self.available_size = Some(new_available_size);
                self.update_scale(ctx);
            }
            
            // Draw checker background
            self.draw_checker_background(ui);

            // Handle keyboard input
            self.handle_input(ui);

            // Display and handle image
            if let Some(texture) = &self.current_image {
                let image_size = Vec2::new(
                    self.image_size.unwrap()[0] as f32,
                    self.image_size.unwrap()[1] as f32,
                );

                let scaled_size = image_size * self.scale;
                let image_rect = Rect::from_center_size(
                    rect.center() + self.offset,
                    scaled_size,
                );

                // Handle dragging
                let response = ui.allocate_rect(rect, Sense::drag());
                if response.dragged() {
                    if !self.dragging {
                        self.drag_start = Some(response.hover_pos().unwrap());
                        self.dragging = true;
                    }
                    if let Some(current_pos) = response.hover_pos() {
                        if let Some(last_pos) = self.last_cursor_pos {
                            let delta = current_pos - last_pos;
                            self.offset += delta;
                        }
                        self.last_cursor_pos = Some(current_pos);
                    }
                } else {
                    self.dragging = false;
                    self.last_cursor_pos = None;
                }

                // Handle zooming
                if let Some(hover_pos) = response.hover_pos() {
                    let zoom_delta = ui.input(|i| i.scroll_delta.y) * 0.001;
                    if zoom_delta != 0.0 {
                        let base_scale = self.scale;
                        let old_scale = base_scale * self.scale;
                        let new_scale = (old_scale * (1.0 + zoom_delta)).max(0.01).min(10.0);
                        self.scale = new_scale / base_scale;
                        
                        // Adjust zoom center
                        let mouse_pos = hover_pos;
                        let center = image_rect.center();
                        let mouse_to_center = mouse_pos - center;
                        let scale_change = new_scale / old_scale;
                        self.offset += mouse_to_center * (1.0 - scale_change);
                        self.update_window_title(ctx);
                    }
                }

                // Draw image
                ui.painter().image(
                    texture.id(),
                    image_rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                // Show "Open Image" button when no image is loaded
                ui.centered_and_justified(|ui| {
                    if ui.button("Open Image").clicked() {
                        self.open_file_dialog = true;
                    }
                });
            }
        });

        // Add bottom bar with image position and info
        if let Some(index) = self.current_image_index {
            egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("Image {}/{}", index + 1, self.image_list.len()));
                    if let Some(size) = self.image_size {
                        ui.separator();
                        ui.label(format!("{}x{}", size[0], size[1]));
                        ui.separator();
                        ui.label(format!("{:.0}%", self.scale * 100.0));
                    }
                });
            });
        }
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_title("Simple Image Viewer"),
        ..Default::default()
    };
    eframe::run_native(
        "Simple Image Viewer",
        options,
        Box::new(|_cc| Box::new(ImageViewer::default())),
    )
}
