//! Document management: files attached to employees, customers, projects or the company.
//! Files are stored on disk under DATA_DIR/documents with random names; only metadata is in MySQL.

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{header, HeaderValue},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    auth::{audit, AuthUser},
    error::{bad, AppError, AppResult},
    state::AppState,
    util::random_token,
};

const MAX_BYTES: usize = 20 * 1024 * 1024;
/// Office documents and images only; nothing a browser would execute.
const ALLOWED: [(&str, &str); 12] = [
    ("pdf", "application/pdf"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
    ("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
    ("pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation"),
    ("doc", "application/msword"),
    ("xls", "application/vnd.ms-excel"),
    ("csv", "text/csv"),
    ("txt", "text/plain"),
    ("zip", "application/zip"),
];
const ENTITIES: [&str; 5] = ["general", "employee", "customer", "project", "expense"];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/documents", get(list).post(upload).layer(DefaultBodyLimit::max(MAX_BYTES + 64 * 1024)))
        .route("/documents/{id}", axum::routing::delete(remove))
        .route("/documents/{id}/download", get(download))
}

fn dir(st: &AppState) -> std::path::PathBuf {
    std::path::Path::new(&st.cfg.data_dir).join("documents")
}

#[derive(Serialize, sqlx::FromRow)]
struct DocRow {
    id: i64,
    title: String,
    category: Option<String>,
    entity: String,
    entity_id: Option<i64>,
    file_name: String,
    content_type: String,
    size_bytes: i64,
    sha256: String,
    uploaded_by_name: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct DocQ {
    entity: Option<String>,
    entity_id: Option<i64>,
}

/// Employees always see the documents attached to themselves.
fn may_view(u: &AuthUser, entity: &str, entity_id: Option<i64>) -> bool {
    u.can("documents.view") || (entity == "employee" && entity_id.is_some() && entity_id == u.0.emp)
}

async fn list(State(st): State<AppState>, u: AuthUser, Query(q): Query<DocQ>) -> AppResult<Json<Vec<DocRow>>> {
    let (entity, entity_id) = if u.can("documents.view") {
        (q.entity, q.entity_id)
    } else {
        (Some("employee".to_string()), Some(u.0.emp.ok_or(AppError::Forbidden)?))
    };
    let mut sql = String::from(
        "SELECT d.id, d.title, d.category, d.entity, d.entity_id, d.file_name, d.content_type, d.size_bytes, d.sha256,
           us.display_name AS uploaded_by_name, d.created_at FROM documents d LEFT JOIN users us ON us.id = d.uploaded_by WHERE 1=1",
    );
    if entity.is_some() {
        sql.push_str(" AND d.entity = ?");
    }
    if entity_id.is_some() {
        sql.push_str(" AND d.entity_id = ?");
    }
    sql.push_str(" ORDER BY d.id DESC LIMIT 1000");
    let mut qq = sqlx::query_as::<_, DocRow>(&sql);
    if let Some(e) = &entity {
        qq = qq.bind(e);
    }
    if let Some(i) = entity_id {
        qq = qq.bind(i);
    }
    Ok(Json(qq.fetch_all(&st.db).await?))
}

async fn upload(State(st): State<AppState>, u: AuthUser, mut mp: Multipart) -> AppResult<Json<Value>> {
    // employees attach receipts to their own expense claims; everything else needs documents.manage
    let manager = u.can("documents.manage");
    let (mut title, mut category, mut entity, mut entity_id) = (String::new(), None, "general".to_string(), None);
    let mut file: Option<(String, Vec<u8>)> = None;
    while let Some(field) = mp.next_field().await.map_err(|e| bad(format!("อัปโหลดไม่สำเร็จ: {e}")))? {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "file" => {
                let fname = field.file_name().unwrap_or("file").to_string();
                let data = field.bytes().await.map_err(|_| bad("ไฟล์ใหญ่เกิน 20 MB"))?;
                file = Some((fname, data.to_vec()));
            }
            _ => {
                let v = field.text().await.unwrap_or_default();
                match name.as_str() {
                    "title" => title = v.trim().to_string(),
                    "category" => category = Some(v.trim().to_string()).filter(|s| !s.is_empty()),
                    "entity" => entity = v.trim().to_string(),
                    "entity_id" => entity_id = v.trim().parse::<i64>().ok(),
                    _ => {}
                }
            }
        }
    }
    let (fname, data) = file.ok_or_else(|| bad("กรุณาเลือกไฟล์"))?;
    if data.is_empty() || data.len() > MAX_BYTES {
        return Err(bad("ไฟล์ว่างหรือใหญ่เกิน 20 MB"));
    }
    if !ENTITIES.contains(&entity.as_str()) {
        return Err(bad("ประเภทเจ้าของเอกสารไม่ถูกต้อง"));
    }
    if entity == "expense" {
        let claim = entity_id.ok_or_else(|| bad("ไม่ระบุใบเบิก"))?;
        if !super::expenses::may_access_receipts(&st, &u, claim, true).await? {
            return Err(bad("แนบใบเสร็จได้เฉพาะใบเบิกของตนเองที่ยังไม่ได้อนุมัติ"));
        }
    } else if !manager {
        return Err(AppError::Forbidden);
    }
    // keep only a safe display name; the stored name is random
    let fname: String = fname.rsplit(['/', '\\']).next().unwrap_or("file").chars().filter(|c| !c.is_control()).take(200).collect();
    let ext = fname.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let ctype = ALLOWED
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, c)| *c)
        .ok_or_else(|| bad("ชนิดไฟล์ไม่รองรับ (รองรับ pdf, รูปภาพ, Office, csv, txt, zip)"))?;
    let sha = hex::encode(Sha256::digest(&data));
    let stored = format!("{}.{ext}", random_token(16));
    let d = dir(&st);
    tokio::fs::create_dir_all(&d).await.map_err(|e| AppError::Internal(e.into()))?;
    tokio::fs::write(d.join(&stored), &data).await.map_err(|e| AppError::Internal(e.into()))?;
    let title = if title.is_empty() { fname.clone() } else { title };
    let id = sqlx::query(
        "INSERT INTO documents (title, category, entity, entity_id, file_name, stored_name, content_type, size_bytes, sha256, uploaded_by)
         VALUES (?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&title)
    .bind(&category)
    .bind(&entity)
    .bind(entity_id)
    .bind(&fname)
    .bind(&stored)
    .bind(ctype)
    .bind(data.len() as i64)
    .bind(&sha)
    .bind(u.id())
    .execute(&st.db)
    .await?
    .last_insert_id();
    audit(&st, Some(&u), "upload", "document", id, json!({ "title": title, "entity": entity, "entity_id": entity_id, "sha256": sha })).await;
    Ok(Json(json!({ "id": id, "sha256": sha })))
}

async fn download(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Response> {
    let (entity, entity_id, fname, stored, ctype): (String, Option<i64>, String, String, String) =
        sqlx::query_as("SELECT entity, entity_id, file_name, stored_name, content_type FROM documents WHERE id = ?")
            .bind(id)
            .fetch_one(&st.db)
            .await?;
    let allowed = if entity == "expense" {
        u.can("documents.view") || super::expenses::may_access_receipts(&st, &u, entity_id.unwrap_or(0), false).await?
    } else {
        may_view(&u, &entity, entity_id)
    };
    if !allowed {
        return Err(AppError::Forbidden);
    }
    let data = tokio::fs::read(dir(&st).join(&stored)).await.map_err(|_| AppError::NotFound)?;
    audit(&st, Some(&u), "download", "document", id, json!({})).await;
    let mut r = Response::new(Body::from(data));
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_str(&ctype).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    // always a download, never rendered inline (no stored XSS through uploaded files)
    let ascii: String = fname.chars().map(|c| if c.is_ascii_alphanumeric() || ".-_".contains(c) { c } else { '_' }).collect();
    let encoded: String = fname.bytes().map(|b| if b.is_ascii_alphanumeric() || b".-_".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")).unwrap_or(HeaderValue::from_static("attachment")),
    );
    Ok(r.into_response())
}

async fn remove(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let row: Option<(String, String, Option<i64>)> =
        sqlx::query_as("SELECT stored_name, entity, entity_id FROM documents WHERE id = ?").bind(id).fetch_optional(&st.db).await?;
    let (stored, entity, entity_id) = row.ok_or(AppError::NotFound)?;
    let own_receipt = entity == "expense" && super::expenses::may_access_receipts(&st, &u, entity_id.unwrap_or(0), true).await?;
    if !(u.can("documents.manage") || own_receipt) {
        return Err(AppError::Forbidden);
    }
    sqlx::query("DELETE FROM documents WHERE id = ?").bind(id).execute(&st.db).await?;
    let _ = tokio::fs::remove_file(dir(&st).join(stored)).await;
    audit(&st, Some(&u), "delete", "document", id, json!({})).await;
    Ok(Json(json!({ "ok": true })))
}
