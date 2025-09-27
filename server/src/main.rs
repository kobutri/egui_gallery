use std::{
    io::{self, BufRead, Read, Seek},
    path::Path,
    str::FromStr,
    sync::{Arc, mpsc},
};

use anyhow::{Context, bail};
use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::Json,
    routing::{any, get, post},
};
use futures_util::{TryStreamExt, future::join_all};
use image::ImageReader;
use serde::{Deserialize, Serialize};
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use tokio::{fs::File, io::BufWriter, sync::Semaphore};
use tokio_util::io::{InspectReader, StreamReader};

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub data_dir: std::path::PathBuf,
}

#[derive(Serialize, Deserialize)]
pub struct ImageResponse {
    pub id: i64,
    pub author: String,
    pub width: i32,
    pub height: i32,
    pub hash: String,
    pub path: String,
    pub mime_type: String,
}

#[derive(Deserialize)]
pub struct FetchImagesQuery {
    pub page: Option<u32>,
    pub limit: Option<u32>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize database
    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::from_str("sqlite::memory")?.create_if_missing(true),
    )
    .await?;
    sqlx::migrate!().run(&pool).await?;

    // Set up data directory
    let Some(dir) = directories::ProjectDirs::from("com", "kobutri", "egui_gallery_backend") else {
        bail!("did not find directories");
    };
    let Some(data_dir) = dir.state_dir() else {
        bail!("did no find state dir");
    };
    std::fs::create_dir_all(data_dir)?;

    let images_dir = data_dir.join("images");
    std::fs::create_dir_all(&images_dir)?;

    println!("Data directory: {:?}", data_dir);

    // Create app state
    let app_state = AppState {
        db: pool,
        data_dir: data_dir.to_path_buf(),
    };

    // Build router
    let app = Router::new()
        .route("/images", get(get_images))
        .route("/images/fetch", any(fetch_and_insert_images))
        .with_state(app_state);

    // Start server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    println!("Server running on http://0.0.0.0:3000");

    axum::serve(listener, app).await?;

    Ok(())
}

// GET /images - Fetch images from database
async fn get_images(
    State(state): State<AppState>,
    Query(params): Query<FetchImagesQuery>,
) -> Result<Json<Vec<ImageResponse>>, StatusCode> {
    let limit = params.limit.unwrap_or(10).min(100) as i64;
    let offset = (params.page.unwrap_or(0) as i64) * limit;

    let images = sqlx::query!(
        "SELECT id, author, width, height, hash, path, mime_type FROM images LIMIT ? OFFSET ?",
        limit,
        offset
    )
    .fetch_all(&state.db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let response: Vec<ImageResponse> = images
        .into_iter()
        .map(|row| ImageResponse {
            id: row.id,
            author: row.author,
            width: row.width as i32,
            height: row.height as i32,
            hash: hex::encode(row.hash),
            path: row.path,
            mime_type: row.mime_type,
        })
        .collect();

    Ok(Json(response))
}

// POST /images/fetch - Fetch images from external API and insert into database
async fn fetch_and_insert_images(
    State(state): State<AppState>,
    Query(params): Query<FetchImagesQuery>,
) -> Result<Json<Vec<ImageResponse>>, StatusCode> {
    let page = params.page.unwrap_or(0);
    let limit = params.limit.unwrap_or(10).min(50);

    println!("pag: {page}, limit: {limit}");

    let images_dir = state.data_dir.join("images");

    let fetched_images = fetch_images(page, limit, &images_dir)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut inserted_images = Vec::new();

    for image_result in fetched_images {
        match image_result {
            Ok(image) => {
                let w = image.width as i32;
                let h = image.height as i32;
                // Insert image into database
                let result = sqlx::query!(
                    r#"INSERT INTO images (author, width, height, hash, path, mime_type) VALUES (?, ?, ?, ?, ?, ?)"#,
                    "Picsum Photos",
                    w,h,
                    image.hash,
                    image.path,
                    image.mime_type
                )
                .execute(&state.db)
                .await;

                match result {
                    Ok(query_result) => {
                        let id = query_result.last_insert_rowid();
                        inserted_images.push(ImageResponse {
                            id,
                            author: "Picsum Photos".to_string(),
                            width: image.width as i32,
                            height: image.height as i32,
                            hash: hex::encode(&image.hash),
                            path: image.path.clone(),
                            mime_type: image.mime_type,
                        });
                    }
                    Err(e) => {
                        eprintln!("Failed to insert image into database: {:?}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to fetch image: {:?}", e);
            }
        }
    }

    Ok(Json(inserted_images))
}

#[derive(Deserialize)]
struct PicsumImage {
    download_url: String,
}

async fn fetch_images(
    page: u32,
    limit: u32,
    path: &Path,
) -> anyhow::Result<Vec<anyhow::Result<Image>>> {
    let images = reqwest::get(format!(
        "https://picsum.photos/v2/list?limit={limit}&page={page}"
    ))
    .await?
    .json::<Vec<PicsumImage>>()
    .await?;

    let semaphore = Arc::new(Semaphore::new(5));

    let images = images.into_iter().map(async |image| {
        let semaphore = semaphore.clone();
        let filename = generate_random_filename();
        let path = path.join(&filename);
        tokio::spawn(async move {
            let lock = semaphore.acquire().await?;

            let reader = reqwest::get(image.download_url)
                .await?
                .bytes_stream()
                .map_err(std::io::Error::other);
            let reader = StreamReader::new(reader);
            let (sender, receiver) = mpsc::channel();
            let mut reader = InspectReader::new(reader, |chunk| {
                if let Err(e) = sender.send(chunk.to_vec()) {
                    eprintln!("{:?}", e);
                }
            });
            let file = File::create(path).await?;
            let mut file = BufWriter::new(file);

            let handle = tokio::task::spawn_blocking(move || {
                let reader = Buffer {
                    buffer: vec![],
                    position: 0,
                    channel: receiver,
                };
                let image = ImageReader::new(reader).with_guessed_format()?;
                let format = image.format().context("couldn't determine format")?;
                let mime_type = format.to_mime_type().to_string();
                let image = image.decode()?;

                let width = image.width();
                let height = image.height();
                let thumbnail = image.thumbnail(100, 100);
                let hash = thumbhash::rgba_to_thumb_hash(
                    thumbnail.width() as usize,
                    thumbnail.height() as usize,
                    &thumbnail.to_rgba8(),
                );

                anyhow::Ok(Image {
                    width,
                    height,
                    hash,
                    path: format!("images/{filename}"),
                    mime_type,
                })
            });
            tokio::io::copy(&mut reader, &mut file).await?;

            let _ = sender.send(vec![]);
            drop(lock);

            let ret = handle.await;

            ret?
        })
        .await?
    });

    Ok(join_all(images).await)
}

struct Image {
    width: u32,
    height: u32,
    hash: Vec<u8>,
    path: String,
    mime_type: String,
}

fn generate_random_filename() -> String {
    let mut bytes = [0u8; 16];
    rand::fill(&mut bytes);
    hex::encode(bytes)
}

struct Buffer {
    buffer: Vec<u8>,
    position: usize,
    channel: mpsc::Receiver<Vec<u8>>,
}

impl Buffer {
    fn read_channel(&mut self) -> io::Result<()> {
        let data = self
            .channel
            .recv()
            .map_err(|_| io::Error::new(io::ErrorKind::UnexpectedEof, "Data has ended"))?;
        self.buffer.extend_from_slice(&data);
        Ok(())
    }
}

impl Read for Buffer {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.buffer.len() {
            self.read_channel()?;
        }
        let mut count = 0;
        for i in 0..(self.buffer.len() - self.position).min(buf.len()) {
            buf[i] = self.buffer[self.position];
            self.position += 1;
            count += 1;
        }
        Ok(count)
    }
}

impl BufRead for Buffer {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.read_channel()?;
        Ok(&self.buffer)
    }

    fn consume(&mut self, amount: usize) {
        self.position = self.buffer.len().min(self.position + amount);
    }
}

impl Seek for Buffer {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            io::SeekFrom::Start(offset) => offset as i64,
            io::SeekFrom::End(_) => unreachable!(),
            io::SeekFrom::Current(offset) => self.position as i64 + offset,
        };

        if new_pos < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid seek"));
        }

        self.position = new_pos as usize;
        Ok(self.position as u64)
    }
}
