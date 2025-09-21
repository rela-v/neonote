use actix_web::{
    body::{BoxBody, EitherBody},
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    web, App, Error, HttpResponse, HttpServer, Responder,
};
use futures_util::future::{ok, LocalBoxFuture, Ready};
use serde::{Deserialize, Serialize};
use sled::Db;
use std::{
    env,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
    time::SystemTime,
};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct CodeLocation {
    file_path: String,
    line_number: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Item {
    id: String,
    #[serde(rename = "type")]
    item_type: String, // e.g., "note", "task", "event", etc.
    title: String,
    content: Option<String>,
    tags: Vec<String>,
    code_location: Option<CodeLocation>,
    created_at: i64,
    completed: Option<bool>,
    due_date: Option<i64>,
    start_time: Option<i64>,
    end_time: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CreateItemPayload {
    #[serde(rename = "type")]
    item_type: String,
    title: String,
    content: Option<String>,
    tags: Option<Vec<String>>,
    code_location: Option<CodeLocation>,
    completed: Option<bool>,
    due_date: Option<i64>,
    start_time: Option<i64>,
    end_time: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateItemPayload {
    #[serde(rename = "type")]
    item_type: Option<String>,
    title: Option<String>,
    content: Option<String>,
    tags: Option<Vec<String>>,
    code_location: Option<CodeLocation>,
    completed: Option<bool>,
    due_date: Option<i64>,
    start_time: Option<i64>,
    end_time: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CapturePayload {
    text: String,
}

type SharedDb = Arc<Db>;

struct ApiKeyMiddleware {
    api_key: String,
}

impl<S, B> Transform<S, ServiceRequest> for ApiKeyMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B, BoxBody>>;
    type Error = Error;
    type Transform = ApiKeyMiddlewareMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(ApiKeyMiddlewareMiddleware {
            service: Rc::new(service),
            api_key: self.api_key.clone(),
        })
    }
}

struct ApiKeyMiddlewareMiddleware<S> {
    service: Rc<S>,
    api_key: String,
}

impl<S, B> Service<ServiceRequest> for ApiKeyMiddlewareMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B, BoxBody>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let api_key = self.api_key.clone();

        if let Some(header_value) = req.headers().get("X-API-Key") {
            if let Ok(value) = header_value.to_str() {
                if value == api_key {
                    let fut = self.service.call(req);
                    return Box::pin(async move {
                        let res = fut.await?;
                        Ok(res.map_into_left_body())
                    });
                }
            }
        }

        let (req, _) = req.into_parts();
        let res = HttpResponse::Unauthorized()
            .body("Missing or invalid API key")
            .map_into_right_body();
        Box::pin(async move { Ok(ServiceResponse::new(req, res)) })
    }
}

async fn list_items(db: web::Data<SharedDb>) -> impl Responder {
    let items: Vec<Item> = db
        .iter()
        .filter_map(|item| {
            if let Ok((_, val)) = item {
                serde_json::from_slice(&val).ok()
            } else {
                None
            }
        })
        .collect();

    HttpResponse::Ok().json(items)
}

async fn get_item(db: web::Data<SharedDb>, path: web::Path<String>) -> impl Responder {
    match db.get(path.into_inner()) {
        Ok(Some(value)) => match serde_json::from_slice::<Item>(&value) {
            Ok(item) => HttpResponse::Ok().json(item),
            Err(_) => HttpResponse::InternalServerError().body("Deserialization failed"),
        },
        Ok(None) => HttpResponse::NotFound().body("Item not found"),
        Err(_) => HttpResponse::InternalServerError().body("DB error"),
    }
}

async fn create_item(
    db: web::Data<SharedDb>,
    payload: web::Json<CreateItemPayload>,
) -> impl Responder {
    let id = Uuid::new_v4().to_string();
    let created_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("Time went backward")
        .as_millis() as i64;
    
    let item = Item {
        id: id.clone(),
        item_type: payload.item_type.clone(),
        title: payload.title.clone(),
        content: payload.content.clone(),
        tags: payload.tags.clone().unwrap_or_default(),
        code_location: payload.code_location.clone(),
        created_at,
        completed: payload.completed,
        due_date: payload.due_date,
        start_time: payload.start_time,
        end_time: payload.end_time,
    };

    match serde_json::to_vec(&item) {
        Ok(bytes) => match db.insert(&id, bytes) {
            Ok(_) => HttpResponse::Created().json(item),
            Err(_) => HttpResponse::InternalServerError().body("Failed to insert"),
        },
        Err(_) => HttpResponse::InternalServerError().body("Serialization failed"),
    }
}

async fn update_item(
    db: web::Data<SharedDb>,
    path: web::Path<String>,
    payload: web::Json<UpdateItemPayload>,
) -> impl Responder {
    let id = path.into_inner();

    match db.get(&id) {
        Ok(Some(value)) => {
            let mut item: Item = serde_json::from_slice(&value).unwrap();

            if let Some(item_type) = &payload.item_type {
                item.item_type = item_type.clone();
            }
            if let Some(title) = &payload.title {
                item.title = title.clone();
            }
            if let Some(content) = &payload.content {
                item.content = Some(content.clone());
            }
            if let Some(tags) = &payload.tags {
                item.tags = tags.clone();
            }
            if let Some(code_location) = &payload.code_location {
                item.code_location = Some(code_location.clone());
            }
            if let Some(completed) = payload.completed {
                item.completed = Some(completed);
            }
            if let Some(due_date) = payload.due_date {
                item.due_date = Some(due_date);
            }
            if let Some(start_time) = payload.start_time {
                item.start_time = Some(start_time);
            }
            if let Some(end_time) = payload.end_time {
                item.end_time = Some(end_time);
            }

            match serde_json::to_vec(&item) {
                Ok(bytes) => {
                    if db.insert(&id, bytes).is_ok() {
                        HttpResponse::Ok().json(item)
                    } else {
                        HttpResponse::InternalServerError().body("Update failed")
                    }
                }
                Err(_) => HttpResponse::InternalServerError().body("Serialization failed"),
            }
        }
        Ok(None) => HttpResponse::NotFound().body("Item not found"),
        Err(_) => HttpResponse::InternalServerError().body("DB error"),
    }
}

async fn delete_item(db: web::Data<SharedDb>, path: web::Path<String>) -> impl Responder {
    let id = path.into_inner();
    match db.remove(&id) {
        Ok(Some(_)) => HttpResponse::NoContent().finish(),
        Ok(None) => HttpResponse::NotFound().body("Item not found"),
        Err(_) => HttpResponse::InternalServerError().body("Delete failed"),
    }
}

async fn capture_item(db: web::Data<SharedDb>, payload: web::Json<CapturePayload>) -> impl Responder {
    let text = payload.text.clone();
    let mut lines = text.lines();
    let first_line = lines.next().unwrap_or("").to_string();
    let content = Some(lines.collect::<Vec<&str>>().join("\n"));

    let mut item_type = "note".to_string();
    let mut tags: Vec<String> = vec![];
    let mut title_parts = Vec::new();

    for word in first_line.split_whitespace() {
        if word.starts_with('#') {
            let tag = word[1..].to_string();
            // Check for special tags to determine item type
            if tag.eq_ignore_ascii_case("todo") {
                item_type = "task".to_string();
            } else if tag.eq_ignore_ascii_case("note") {
                item_type = "note".to_string();
            } else if tag.eq_ignore_ascii_case("event") {
                item_type = "event".to_string();
            }
            tags.push(tag);
        } else {
            title_parts.push(word);
        }
    }
    
    let title = title_parts.join(" ");

    let id = Uuid::new_v4().to_string();
    let created_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("Time went backward")
        .as_millis() as i64;
    
    let item = Item {
        id: id.clone(),
        item_type,
        title: title.trim().to_string(),
        content,
        tags,
        code_location: None,
        created_at,
        completed: None,
        due_date: None,
        start_time: None,
        end_time: None,
    };

    match serde_json::to_vec(&item) {
        Ok(bytes) => match db.insert(&id, bytes) {
            Ok(_) => HttpResponse::Created().json(item),
            Err(_) => HttpResponse::InternalServerError().body("Failed to insert item"),
        },
        Err(_) => HttpResponse::InternalServerError().body("Serialization failed"),
    }
}

async fn get_filtered_items(
    db: web::Data<SharedDb>,
    info: web::Query<std::collections::HashMap<String, String>>,
) -> impl Responder {
    let filter_type = info.get("type").map(|s| s.to_lowercase());
    let filter_tags: Option<Vec<String>> = info.get("tags").map(|s| {
        s.split(',')
            .map(|tag| tag.trim().to_string())
            .collect()
    });

    let items: Vec<Item> = db
        .iter()
        .filter_map(|item| {
            if let Ok((_, val)) = item {
                let item_data: Item = serde_json::from_slice(&val).ok()?;

                let type_match = filter_type.as_ref().map_or(true, |t| {
                    t == &item_data.item_type
                });
                
                let tags_match = filter_tags.as_ref().map_or(true, |tags| {
                    tags.iter().all(|tag| item_data.tags.contains(tag))
                });
                
                if type_match && tags_match {
                    return Some(item_data);
                }
            }
            None
        })
        .collect();

    HttpResponse::Ok().json(items)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let api_key = env::var("API_KEY").unwrap_or_else(|_| "secret".into());
    let db = sled::open("/usr/src/app/data/notes_db").expect("Failed to open sled database");
    let shared_db = web::Data::new(Arc::new(db));

    println!("Server running at http://localhost:8080");

    HttpServer::new(move || {
        App::new()
            .app_data(shared_db.clone())
            .wrap(ApiKeyMiddleware {
                api_key: api_key.clone(),
            })
            .service(
                web::scope("/items")
                    .route("/capture", web::post().to(capture_item))
                    .route("", web::get().to(get_filtered_items))
                    .route("", web::post().to(create_item))
                    .route("/{id}", web::get().to(get_item))
                    .route("/{id}", web::put().to(update_item))
                    .route("/{id}", web::delete().to(delete_item)),
            )
    })
    .bind(("0.0.0.0", 8080))?
    .run()
    .await
}

