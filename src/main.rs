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
};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Note {
    id: String,
    title: String,
    content: String,
    tags: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CreateNotePayload {
    title: String,
    content: String,
    tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateNotePayload {
    title: Option<String>,
    content: Option<String>,
    tags: Option<Vec<String>>,
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
    // Use EitherBody for response type here
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

    // Forward readiness check to inner service
    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let api_key = self.api_key.clone();

        // Check for X-API-Key header
        if let Some(header_value) = req.headers().get("X-API-Key") {
            if let Ok(value) = header_value.to_str() {
                if value == api_key {
                    let fut = self.service.call(req);

                    // Call inner service and map response body to left side of EitherBody
                    return Box::pin(async move {
                        let res = fut.await?;
                        Ok(res.map_into_left_body())
                    });
                }
            }
        }

        // Unauthorized response if missing/invalid key
        let (req, _) = req.into_parts();

        let res = HttpResponse::Unauthorized()
            .body("Missing or invalid API key")
            .map_into_right_body();

        Box::pin(async move { Ok(ServiceResponse::new(req, res)) })
    }
}

// ------------------------- Handlers -------------------------

async fn list_notes(db: web::Data<SharedDb>) -> impl Responder {
    let notes: Vec<Note> = db
        .iter()
        .filter_map(|item| {
            if let Ok((_, val)) = item {
                serde_json::from_slice(&val).ok()
            } else {
                None
            }
        })
        .collect();

    HttpResponse::Ok().json(notes)
}

async fn get_note(db: web::Data<SharedDb>, path: web::Path<String>) -> impl Responder {
    match db.get(path.into_inner()) {
        Ok(Some(value)) => match serde_json::from_slice::<Note>(&value) {
            Ok(note) => HttpResponse::Ok().json(note),
            Err(_) => HttpResponse::InternalServerError().body("Deserialization failed"),
        },
        Ok(None) => HttpResponse::NotFound().body("Note not found"),
        Err(_) => HttpResponse::InternalServerError().body("DB error"),
    }
}

async fn create_note(
    db: web::Data<SharedDb>,
    payload: web::Json<CreateNotePayload>,
) -> impl Responder {
    let id = Uuid::new_v4().to_string();
    let note = Note {
        id: id.clone(),
        title: payload.title.clone(),
        content: payload.content.clone(),
        tags: payload.tags.clone().unwrap_or_default(),
    };

    match serde_json::to_vec(&note) {
        Ok(bytes) => match db.insert(&id, bytes) {
            Ok(_) => HttpResponse::Created().json(note),
            Err(_) => HttpResponse::InternalServerError().body("Failed to insert"),
        },
        Err(_) => HttpResponse::InternalServerError().body("Serialization failed"),
    }
}

async fn update_note(
    db: web::Data<SharedDb>,
    path: web::Path<String>,
    payload: web::Json<UpdateNotePayload>,
) -> impl Responder {
    let id = path.into_inner();

    match db.get(&id) {
        Ok(Some(value)) => {
            let mut note: Note = serde_json::from_slice(&value).unwrap_or_else(|_| Note {
                id: id.clone(),
                title: "".into(),
                content: "".into(),
                tags: vec![],
            });

            if let Some(title) = &payload.title {
                note.title = title.clone();
            }
            if let Some(content) = &payload.content {
                note.content = content.clone();
            }
            if let Some(tags) = &payload.tags {
                note.tags = tags.clone();
            }

            match serde_json::to_vec(&note) {
                Ok(bytes) => {
                    if db.insert(&id, bytes).is_ok() {
                        HttpResponse::Ok().json(note)
                    } else {
                        HttpResponse::InternalServerError().body("Update failed")
                    }
                }
                Err(_) => HttpResponse::InternalServerError().body("Serialization failed"),
            }
        }
        Ok(None) => HttpResponse::NotFound().body("Note not found"),
        Err(_) => HttpResponse::InternalServerError().body("DB error"),
    }
}

async fn delete_note(db: web::Data<SharedDb>, path: web::Path<String>) -> impl Responder {
    let id = path.into_inner();
    match db.remove(&id) {
        Ok(Some(_)) => HttpResponse::NoContent().finish(),
        Ok(None) => HttpResponse::NotFound().body("Note not found"),
        Err(_) => HttpResponse::InternalServerError().body("Delete failed"),
    }
}

async fn get_notes_by_tag(db: web::Data<SharedDb>, path: web::Path<String>) -> impl Responder {
    let tag = path.into_inner();
    let notes: Vec<Note> = db
        .iter()
        .filter_map(|item| {
            if let Ok((_, val)) = item {
                let note: Note = serde_json::from_slice(&val).ok()?;
                if note.tags.contains(&tag) {
                    return Some(note);
                }
            }
            None
        })
        .collect();

    HttpResponse::Ok().json(notes)
}

// ------------------------- Main -------------------------

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let api_key = env::var("API_KEY").unwrap_or_else(|_| "secret".into());
    let db = sled::open("notes_db").expect("Failed to open sled database");
    let shared_db = web::Data::new(Arc::new(db));

    println!("Server running at http://localhost:8080");

    HttpServer::new(move || {
        App::new()
            .app_data(shared_db.clone())
            .wrap(ApiKeyMiddleware {
                api_key: api_key.clone(),
            })
            .service(
                web::scope("/notes")
                    .route("/tags/{tag}", web::get().to(get_notes_by_tag))
                    .route("", web::get().to(list_notes))
                    .route("", web::post().to(create_note))
                    .route("/{id}", web::get().to(get_note))
                    .route("/{id}", web::put().to(update_note))
                    .route("/{id}", web::delete().to(delete_note)),
            )
    })
    .bind(("0.0.0.0", 8080))?
    .run()
    .await
}

