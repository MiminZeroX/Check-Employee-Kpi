//! Products / services, warehouses and stock with moving-average cost.
//! Stock changes only through `stock_in` / `stock_out` / `reverse_moves`, always inside
//! the caller's transaction and with the product row locked, so balances, the stock card
//! and the average cost stay consistent under concurrent documents.

use axum::{
    extract::{Path, Query, State},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    auth::{audit, AuthUser},
    erp::{self, r2, Acc, Line, Tx},
    error::{bad, AppError, AppResult},
    state::AppState,
    util::local_today,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/products", get(list_products).post(create_product))
        .route("/products/{id}", put(update_product))
        .route("/products/{id}/stock", get(product_stock))
        .route("/warehouses", get(list_warehouses).post(create_warehouse))
        .route("/warehouses/{id}", put(update_warehouse))
        .route("/inventory/stock", get(stock_report))
        .route("/inventory/adjust", post(adjust))
        .route("/inventory/transfer", post(transfer))
}

const PRODUCT_EDITORS: [&str; 4] = ["sales.manage", "purchasing.manage", "inventory.manage", "accounting.manage"];

// ---------------------------------------------------------------- stock engine

fn r4(v: Decimal) -> Decimal {
    v.round_dp_with_strategy(4, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
}

async fn lock_product(tx: &mut Tx<'_>, product: i64) -> AppResult<(Decimal, bool, String)> {
    let r: Option<(Decimal, bool, String)> = sqlx::query_as("SELECT avg_cost, track_stock, name FROM products WHERE id = ? FOR UPDATE")
        .bind(product)
        .fetch_optional(&mut **tx)
        .await?;
    r.ok_or_else(|| bad("ไม่พบสินค้า"))
}

async fn total_qty(tx: &mut Tx<'_>, product: i64) -> AppResult<Decimal> {
    Ok(sqlx::query_scalar("SELECT COALESCE(SUM(qty), 0) FROM stock_balances WHERE product_id = ?")
        .bind(product)
        .fetch_one(&mut **tx)
        .await?)
}

async fn add_balance(tx: &mut Tx<'_>, product: i64, wh: i64, qty: Decimal) -> AppResult<()> {
    sqlx::query("INSERT INTO stock_balances (product_id, warehouse_id, qty) VALUES (?,?,?) ON DUPLICATE KEY UPDATE qty = qty + VALUES(qty)")
        .bind(product)
        .bind(wh)
        .bind(qty)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub struct Move<'a> {
    pub date: NaiveDate,
    pub product: i64,
    pub warehouse: i64,
    pub qty: Decimal,
    pub source: &'a str,
    pub source_id: Option<i64>,
    pub note: Option<String>,
    pub user: Option<i64>,
}

async fn insert_move(tx: &mut Tx<'_>, m: &Move<'_>, signed_qty: Decimal, unit_cost: Decimal) -> AppResult<Decimal> {
    let value = r2(signed_qty * unit_cost);
    sqlx::query(
        "INSERT INTO stock_moves (move_date, product_id, warehouse_id, qty, unit_cost, value, source, source_id, note, created_by)
         VALUES (?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(m.date)
    .bind(m.product)
    .bind(m.warehouse)
    .bind(signed_qty)
    .bind(r4(unit_cost))
    .bind(value)
    .bind(m.source)
    .bind(m.source_id)
    .bind(&m.note)
    .bind(m.user)
    .execute(&mut **tx)
    .await?;
    Ok(value)
}

/// Receives stock at `unit_cost`; returns the value added.
pub async fn stock_in(tx: &mut Tx<'_>, m: Move<'_>, unit_cost: Decimal) -> AppResult<Decimal> {
    if m.qty <= Decimal::ZERO {
        return Err(bad("จำนวนต้องมากกว่า 0"));
    }
    let (avg, _, _) = lock_product(tx, m.product).await?;
    let on_hand = total_qty(tx, m.product).await?.max(Decimal::ZERO);
    let new_avg = if on_hand + m.qty > Decimal::ZERO { (on_hand * avg + m.qty * unit_cost) / (on_hand + m.qty) } else { unit_cost };
    sqlx::query("UPDATE products SET avg_cost = ? WHERE id = ?").bind(r4(new_avg)).bind(m.product).execute(&mut **tx).await?;
    add_balance(tx, m.product, m.warehouse, m.qty).await?;
    insert_move(tx, &m, m.qty, unit_cost).await
}

/// Issues stock at the current average cost; refuses to go below zero. Returns the cost value.
pub async fn stock_out(tx: &mut Tx<'_>, m: Move<'_>) -> AppResult<Decimal> {
    if m.qty <= Decimal::ZERO {
        return Err(bad("จำนวนต้องมากกว่า 0"));
    }
    let (avg, _, name) = lock_product(tx, m.product).await?;
    let have: Decimal = sqlx::query_scalar("SELECT COALESCE(MAX(qty), 0) FROM stock_balances WHERE product_id = ? AND warehouse_id = ?")
        .bind(m.product)
        .bind(m.warehouse)
        .fetch_one(&mut **tx)
        .await?;
    if have < m.qty {
        return Err(bad(format!("สินค้า \"{name}\" คงเหลือในคลังไม่พอ (มี {}, ต้องการ {})", have.normalize(), m.qty.normalize())));
    }
    add_balance(tx, m.product, m.warehouse, -m.qty).await?;
    Ok(-insert_move(tx, &m, -m.qty, avg).await?)
}

/// Undoes every stock movement of a document (when it is voided). Returns the total
/// value that was reversed (positive = stock came back in).
pub async fn reverse_moves(tx: &mut Tx<'_>, source: &str, source_id: i64, reverse_source: &str, date: NaiveDate, user: Option<i64>) -> AppResult<Decimal> {
    let moves: Vec<(i64, i64, Decimal, Decimal)> =
        sqlx::query_as("SELECT product_id, warehouse_id, qty, unit_cost FROM stock_moves WHERE source = ? AND source_id = ?")
            .bind(source)
            .bind(source_id)
            .fetch_all(&mut **tx)
            .await?;
    let mut total = Decimal::ZERO;
    for (product, wh, qty, cost) in moves {
        let m = Move { date, product, warehouse: wh, qty: qty.abs(), source: reverse_source, source_id: Some(source_id), note: None, user };
        if qty < Decimal::ZERO {
            // an issue comes back in at the cost it left with
            total += stock_in(tx, m, cost).await?;
        } else {
            // a receipt goes back out at its own cost; the average cost is recalculated without it
            let (avg, _, name) = lock_product(tx, product).await?;
            let on_hand = total_qty(tx, product).await?;
            let have: Decimal = sqlx::query_scalar("SELECT COALESCE(MAX(qty), 0) FROM stock_balances WHERE product_id = ? AND warehouse_id = ?")
                .bind(product)
                .bind(wh)
                .fetch_one(&mut **tx)
                .await?;
            if have < qty {
                return Err(bad(format!("ยกเลิกไม่ได้: สินค้า \"{name}\" ถูกใช้ไปแล้ว คงเหลือ {} จาก {}", have.normalize(), qty.normalize())));
            }
            let remaining = on_hand - qty;
            if remaining > Decimal::ZERO {
                let new_avg = ((on_hand * avg - qty * cost) / remaining).max(Decimal::ZERO);
                sqlx::query("UPDATE products SET avg_cost = ? WHERE id = ?").bind(r4(new_avg)).bind(product).execute(&mut **tx).await?;
            }
            add_balance(tx, product, wh, -qty).await?;
            total += insert_move(tx, &m, -qty, cost).await?;
        }
    }
    Ok(total)
}

pub async fn default_warehouse(db: impl sqlx::MySqlExecutor<'_>) -> AppResult<i64> {
    sqlx::query_scalar("SELECT id FROM warehouses WHERE is_active = 1 ORDER BY is_default DESC, id LIMIT 1")
        .fetch_optional(db)
        .await?
        .ok_or_else(|| bad("ยังไม่มีคลังสินค้า"))
}

// ---------------------------------------------------------------- products

#[derive(Serialize, sqlx::FromRow)]
pub struct Product {
    pub id: i64,
    pub sku: String,
    pub name: String,
    pub kind: String,
    pub unit: String,
    pub sale_price: Decimal,
    pub purchase_price: Decimal,
    pub avg_cost: Decimal,
    pub vat_applicable: bool,
    pub track_stock: bool,
    pub reorder_point: Decimal,
    pub revenue_account_id: Option<i64>,
    pub expense_account_id: Option<i64>,
    pub is_active: bool,
    pub on_hand: Decimal,
}

const PRODUCT_SELECT: &str = "SELECT p.id, p.sku, p.name, p.kind, p.unit, p.sale_price, p.purchase_price, p.avg_cost, p.vat_applicable,
    p.track_stock, p.reorder_point, p.revenue_account_id, p.expense_account_id, p.is_active,
    COALESCE((SELECT SUM(b.qty) FROM stock_balances b WHERE b.product_id = p.id), 0) AS on_hand FROM products p";

async fn list_products(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<Product>>> {
    u.need_any(&["sales.view", "sales.manage", "purchasing.view", "purchasing.manage", "inventory.manage", "accounting.view"])?;
    Ok(Json(sqlx::query_as(&format!("{PRODUCT_SELECT} ORDER BY p.sku")).fetch_all(&st.db).await?))
}

#[derive(Deserialize)]
struct ProductReq {
    sku: String,
    name: String,
    kind: String,
    unit: Option<String>,
    #[serde(default)]
    sale_price: Decimal,
    #[serde(default)]
    purchase_price: Decimal,
    vat_applicable: Option<bool>,
    track_stock: Option<bool>,
    #[serde(default)]
    reorder_point: Decimal,
    revenue_account_id: Option<i64>,
    expense_account_id: Option<i64>,
    is_active: Option<bool>,
    /// opening stock (new products only)
    #[serde(default)]
    opening_qty: Decimal,
    opening_cost: Option<Decimal>,
}

async fn validate_product(st: &AppState, r: &ProductReq) -> AppResult<()> {
    let sku = r.sku.trim();
    if sku.is_empty() || sku.len() > 40 || r.name.trim().is_empty() {
        return Err(bad("กรุณาระบุรหัสสินค้า (ไม่เกิน 40 ตัว) และชื่อ"));
    }
    if !["product", "service"].contains(&r.kind.as_str()) {
        return Err(bad("ชนิดต้องเป็นสินค้าหรือบริการ"));
    }
    if r.sale_price < Decimal::ZERO || r.purchase_price < Decimal::ZERO || r.reorder_point < Decimal::ZERO || r.opening_qty < Decimal::ZERO {
        return Err(bad("ราคา / จำนวนต้องไม่ติดลบ"));
    }
    for (acc, want) in [(r.revenue_account_id, "revenue"), (r.expense_account_id, "")] {
        if let Some(a) = acc {
            let t: Option<String> = sqlx::query_scalar("SELECT type FROM accounts WHERE id = ? AND is_active = 1").bind(a).fetch_optional(&st.db).await?;
            match t.as_deref() {
                Some(t) if want.is_empty() && (t == "expense" || t == "asset") => {}
                Some(t) if t == want => {}
                _ => return Err(bad("บัญชีที่ผูกกับสินค้าไม่ถูกต้อง")),
            }
        }
    }
    Ok(())
}

async fn create_product(State(st): State<AppState>, u: AuthUser, Json(r): Json<ProductReq>) -> AppResult<Json<Value>> {
    u.need_any(&PRODUCT_EDITORS)?;
    validate_product(&st, &r).await?;
    let track = r.kind == "product" && r.track_stock.unwrap_or(true);
    let mut tx = st.db.begin().await?;
    let id = sqlx::query(
        "INSERT INTO products (sku, name, kind, unit, sale_price, purchase_price, vat_applicable, track_stock, reorder_point,
            revenue_account_id, expense_account_id, is_active) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(r.sku.trim())
    .bind(r.name.trim())
    .bind(&r.kind)
    .bind(r.unit.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(if r.kind == "service" { "งาน" } else { "ชิ้น" }))
    .bind(r.sale_price)
    .bind(r.purchase_price)
    .bind(r.vat_applicable.unwrap_or(true))
    .bind(track)
    .bind(r.reorder_point)
    .bind(r.revenue_account_id)
    .bind(r.expense_account_id)
    .bind(r.is_active.unwrap_or(true))
    .execute(&mut *tx)
    .await?
    .last_insert_id() as i64;
    if track && r.opening_qty > Decimal::ZERO {
        // opening stock: Dr inventory, Cr opening balance equity
        let cost = r.opening_cost.unwrap_or(r.purchase_price);
        let date = local_today(st.cfg.tz_offset_min);
        let wh = default_warehouse(&mut *tx).await?;
        let value = stock_in(
            &mut tx,
            Move { date, product: id, warehouse: wh, qty: r.opening_qty, source: "opening", source_id: Some(id), note: Some("ยอดยกมา".into()), user: Some(u.id()) },
            cost,
        )
        .await?;
        if value > Decimal::ZERO {
            erp::post(&mut tx, date, &format!("สินค้ายกมา {} {}", r.sku.trim(), r.name.trim()), "stock_adjust", Some(id),
                vec![Line::dr(Acc::Role("inventory"), value), Line::cr(Acc::Role("capital"), value)], Some(u.id())).await?;
        }
    }
    tx.commit().await?;
    audit(&st, Some(&u), "create", "product", id, json!({ "sku": r.sku, "name": r.name })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_product(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<ProductReq>) -> AppResult<Json<Value>> {
    u.need_any(&PRODUCT_EDITORS)?;
    validate_product(&st, &r).await?;
    let cur: Option<(bool, String)> = sqlx::query_as("SELECT track_stock, kind FROM products WHERE id = ?").bind(id).fetch_optional(&st.db).await?;
    let (was_tracked, _) = cur.ok_or(AppError::NotFound)?;
    let track = r.kind == "product" && r.track_stock.unwrap_or(true);
    if was_tracked != track {
        let moves: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stock_moves WHERE product_id = ?").bind(id).fetch_one(&st.db).await?;
        if moves > 0 {
            return Err(bad("สินค้านี้มีความเคลื่อนไหวในคลังแล้ว เปลี่ยนการนับสต็อกไม่ได้"));
        }
    }
    sqlx::query(
        "UPDATE products SET sku = ?, name = ?, kind = ?, unit = ?, sale_price = ?, purchase_price = ?, vat_applicable = ?, track_stock = ?,
            reorder_point = ?, revenue_account_id = ?, expense_account_id = ?, is_active = ? WHERE id = ?",
    )
    .bind(r.sku.trim())
    .bind(r.name.trim())
    .bind(&r.kind)
    .bind(r.unit.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("ชิ้น"))
    .bind(r.sale_price)
    .bind(r.purchase_price)
    .bind(r.vat_applicable.unwrap_or(true))
    .bind(track)
    .bind(r.reorder_point)
    .bind(r.revenue_account_id)
    .bind(r.expense_account_id)
    .bind(r.is_active.unwrap_or(true))
    .bind(id)
    .execute(&st.db)
    .await?;
    audit(&st, Some(&u), "update", "product", id, json!({ "sku": r.sku })).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize, sqlx::FromRow)]
struct MoveRow {
    id: i64,
    move_date: NaiveDate,
    warehouse: String,
    qty: Decimal,
    unit_cost: Decimal,
    value: Decimal,
    source: String,
    source_id: Option<i64>,
    note: Option<String>,
    created_at: DateTime<Utc>,
}

/// Stock card of one product.
async fn product_stock(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    u.need_any(&["sales.view", "purchasing.view", "inventory.manage", "accounting.view", "sales.manage", "purchasing.manage"])?;
    let p: Product = sqlx::query_as(&format!("{PRODUCT_SELECT} WHERE p.id = ?")).bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let balances: Vec<(i64, String, String, Decimal)> = sqlx::query_as(
        "SELECT w.id, w.code, w.name, COALESCE(b.qty, 0) FROM warehouses w LEFT JOIN stock_balances b ON b.warehouse_id = w.id AND b.product_id = ?
          WHERE w.is_active = 1 ORDER BY w.is_default DESC, w.code",
    )
    .bind(id)
    .fetch_all(&st.db)
    .await?;
    let moves: Vec<MoveRow> = sqlx::query_as(
        "SELECT m.id, m.move_date, w.name AS warehouse, m.qty, m.unit_cost, m.value, m.source, m.source_id, m.note, m.created_at
           FROM stock_moves m JOIN warehouses w ON w.id = m.warehouse_id WHERE m.product_id = ? ORDER BY m.move_date DESC, m.id DESC LIMIT 500",
    )
    .bind(id)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(json!({
        "product": p, "moves": moves,
        "balances": balances.into_iter().map(|b| json!({ "warehouse_id": b.0, "code": b.1, "name": b.2, "qty": b.3 })).collect::<Vec<_>>(),
    })))
}

// ---------------------------------------------------------------- warehouses

#[derive(Serialize, sqlx::FromRow)]
struct Warehouse {
    id: i64,
    code: String,
    name: String,
    branch_id: Option<i64>,
    is_default: bool,
    is_active: bool,
}

async fn list_warehouses(State(st): State<AppState>, u: AuthUser) -> AppResult<Json<Vec<Warehouse>>> {
    u.need_any(&["sales.view", "sales.manage", "purchasing.view", "purchasing.manage", "inventory.manage", "accounting.view"])?;
    Ok(Json(
        sqlx::query_as("SELECT id, code, name, branch_id, is_default, is_active FROM warehouses ORDER BY is_default DESC, code").fetch_all(&st.db).await?,
    ))
}

#[derive(Deserialize)]
struct WarehouseReq {
    code: String,
    name: String,
    branch_id: Option<i64>,
    is_default: Option<bool>,
    is_active: Option<bool>,
}

async fn save_warehouse(st: &AppState, id: Option<i64>, r: &WarehouseReq) -> AppResult<i64> {
    if r.code.trim().is_empty() || r.code.len() > 20 || r.name.trim().is_empty() {
        return Err(bad("กรุณาระบุรหัสคลัง (ไม่เกิน 20 ตัว) และชื่อ"));
    }
    let mut tx = st.db.begin().await?;
    if r.is_default == Some(true) {
        sqlx::query("UPDATE warehouses SET is_default = 0").execute(&mut *tx).await?;
    }
    let id = match id {
        Some(id) => {
            sqlx::query("UPDATE warehouses SET code = ?, name = ?, branch_id = ?, is_default = ?, is_active = ? WHERE id = ?")
                .bind(r.code.trim())
                .bind(r.name.trim())
                .bind(r.branch_id)
                .bind(r.is_default.unwrap_or(false))
                .bind(r.is_active.unwrap_or(true))
                .bind(id)
                .execute(&mut *tx)
                .await?;
            id
        }
        None => sqlx::query("INSERT INTO warehouses (code, name, branch_id, is_default, is_active) VALUES (?,?,?,?,?)")
            .bind(r.code.trim())
            .bind(r.name.trim())
            .bind(r.branch_id)
            .bind(r.is_default.unwrap_or(false))
            .bind(r.is_active.unwrap_or(true))
            .execute(&mut *tx)
            .await?
            .last_insert_id() as i64,
    };
    tx.commit().await?;
    Ok(id)
}

async fn create_warehouse(State(st): State<AppState>, u: AuthUser, Json(r): Json<WarehouseReq>) -> AppResult<Json<Value>> {
    u.need("inventory.manage")?;
    let id = save_warehouse(&st, None, &r).await?;
    audit(&st, Some(&u), "create", "warehouse", id, json!({ "code": r.code })).await;
    Ok(Json(json!({ "id": id })))
}

async fn update_warehouse(State(st): State<AppState>, u: AuthUser, Path(id): Path<i64>, Json(r): Json<WarehouseReq>) -> AppResult<Json<Value>> {
    u.need("inventory.manage")?;
    save_warehouse(&st, Some(id), &r).await?;
    audit(&st, Some(&u), "update", "warehouse", id, json!({ "code": r.code })).await;
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- stock report, adjustment, transfer

#[derive(Deserialize)]
struct StockQ {
    warehouse_id: Option<i64>,
    low: Option<bool>,
}

async fn stock_report(State(st): State<AppState>, u: AuthUser, Query(q): Query<StockQ>) -> AppResult<Json<Value>> {
    u.need_any(&["inventory.manage", "purchasing.view", "purchasing.manage", "sales.view", "accounting.view"])?;
    let rows: Vec<(i64, String, String, String, Decimal, Decimal, Decimal)> = sqlx::query_as(
        "SELECT p.id, p.sku, p.name, p.unit, p.avg_cost, p.reorder_point,
                COALESCE((SELECT SUM(b.qty) FROM stock_balances b WHERE b.product_id = p.id AND (? IS NULL OR b.warehouse_id = ?)), 0) AS qty
           FROM products p WHERE p.track_stock = 1 AND p.is_active = 1 ORDER BY p.sku",
    )
    .bind(q.warehouse_id)
    .bind(q.warehouse_id)
    .fetch_all(&st.db)
    .await?;
    let items: Vec<Value> = rows
        .into_iter()
        .filter(|r| q.low != Some(true) || (r.5 > Decimal::ZERO && r.6 <= r.5))
        .map(|r| json!({ "id": r.0, "sku": r.1, "name": r.2, "unit": r.3, "avg_cost": r.4, "reorder_point": r.5, "qty": r.6,
                        "value": r2(r.6 * r.4), "low": r.5 > Decimal::ZERO && r.6 <= r.5 }))
        .collect();
    let total: f64 = items.iter().map(|i| i["value"].as_f64().unwrap_or(0.0)).sum();
    Ok(Json(json!({ "items": items, "total_value": total })))
}

#[derive(Deserialize)]
struct AdjustReq {
    product_id: i64,
    warehouse_id: i64,
    /// counted quantity (the system books the difference)
    counted_qty: Decimal,
    date: Option<NaiveDate>,
    note: Option<String>,
}

/// Stock count: books the difference between counted and system quantity
/// (gain / loss to "stock adjustment").
async fn adjust(State(st): State<AppState>, u: AuthUser, Json(r): Json<AdjustReq>) -> AppResult<Json<Value>> {
    u.need("inventory.manage")?;
    if r.counted_qty < Decimal::ZERO {
        return Err(bad("จำนวนที่นับได้ต้องไม่ติดลบ"));
    }
    let date = r.date.unwrap_or_else(|| local_today(st.cfg.tz_offset_min));
    let mut tx = st.db.begin().await?;
    let (avg, track, name) = lock_product(&mut tx, r.product_id).await?;
    if !track {
        return Err(bad("สินค้านี้ไม่ได้นับสต็อก"));
    }
    let have: Decimal = sqlx::query_scalar("SELECT COALESCE(MAX(qty), 0) FROM stock_balances WHERE product_id = ? AND warehouse_id = ?")
        .bind(r.product_id)
        .bind(r.warehouse_id)
        .fetch_one(&mut *tx)
        .await?;
    let diff = r.counted_qty - have;
    if diff.is_zero() {
        return Ok(Json(json!({ "ok": true, "difference": 0 })));
    }
    let note = Some(r.note.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "ตรวจนับสต็อก".into()));
    let m = Move { date, product: r.product_id, warehouse: r.warehouse_id, qty: diff.abs(), source: "adjustment", source_id: None, note, user: Some(u.id()) };
    let value = if diff > Decimal::ZERO { stock_in(&mut tx, m, avg).await? } else { -stock_out(&mut tx, m).await? };
    let memo = format!("ปรับปรุงสต็อก {name} {} → {}", have.normalize(), r.counted_qty.normalize());
    if !value.is_zero() {
        erp::post(&mut tx, date, &memo, "stock_adjust", Some(r.product_id),
            vec![Line::dr(Acc::Role("inventory"), value), Line::cr(Acc::Role("stock_adjustment"), value)], Some(u.id())).await?;
    }
    tx.commit().await?;
    audit(&st, Some(&u), "adjust", "stock", r.product_id, json!({ "warehouse_id": r.warehouse_id, "from": have, "to": r.counted_qty })).await;
    Ok(Json(json!({ "ok": true, "difference": diff, "value": value })))
}

#[derive(Deserialize)]
struct TransferReq {
    product_id: i64,
    from_warehouse_id: i64,
    to_warehouse_id: i64,
    qty: Decimal,
    date: Option<NaiveDate>,
    note: Option<String>,
}

async fn transfer(State(st): State<AppState>, u: AuthUser, Json(r): Json<TransferReq>) -> AppResult<Json<Value>> {
    u.need("inventory.manage")?;
    if r.from_warehouse_id == r.to_warehouse_id {
        return Err(bad("คลังต้นทางและปลายทางต้องต่างกัน"));
    }
    let date = r.date.unwrap_or_else(|| local_today(st.cfg.tz_offset_min));
    let mut tx = st.db.begin().await?;
    let (avg, _, _) = lock_product(&mut tx, r.product_id).await?;
    let note = r.note.clone().filter(|n| !n.trim().is_empty());
    stock_out(&mut tx, Move { date, product: r.product_id, warehouse: r.from_warehouse_id, qty: r.qty, source: "transfer", source_id: None, note: note.clone(), user: Some(u.id()) }).await?;
    stock_in(&mut tx, Move { date, product: r.product_id, warehouse: r.to_warehouse_id, qty: r.qty, source: "transfer", source_id: None, note, user: Some(u.id()) }, avg).await?;
    tx.commit().await?;
    audit(&st, Some(&u), "transfer", "stock", r.product_id, json!({ "from": r.from_warehouse_id, "to": r.to_warehouse_id, "qty": r.qty })).await;
    Ok(Json(json!({ "ok": true })))
}
