use napi_derive::napi;

#[napi]
pub struct NativeSession {
    session: spotcore_ffi::Session,
}

#[napi]
impl NativeSession {
    #[napi(constructor)]
    pub fn new() -> napi::Result<Self> {
        spotcore_ffi::Session::new()
            .map(|session| Self { session })
            .map_err(|error| napi::Error::new(napi::Status::GenericFailure, error.to_string()))
    }

    #[napi]
    pub async fn request(&self, json: String) -> String {
        spotcore_ffi::wire::call(&self.session, &json).await
    }
}
