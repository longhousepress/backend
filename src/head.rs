use rocket::http::Status;

#[head("/")]
pub async fn head() -> Status {
    Status::Ok
}
