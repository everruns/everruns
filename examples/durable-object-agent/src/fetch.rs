//! Object-local fetch transport for the ordinary shared provider protocols.
use async_trait::async_trait;
use everruns_contracts::http_transport::{
    HeaderMap, HttpError, HttpRequest, HttpResponse, HttpTransport, StatusCode, header,
};
use everruns_contracts::rt::AssertSend;
use js_sys::{Function, Reflect, Uint8Array};
use serde_json::json;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

pub struct FetchTransport(pub AssertSend<Function>);
struct FetchGuard {
    controller: web_sys::AbortController,
    reader: Option<web_sys::ReadableStreamDefaultReader>,
}
impl Drop for FetchGuard {
    fn drop(&mut self) {
        self.controller.abort();
        if let Some(reader) = self.reader.take() {
            let promise = reader.cancel();
            wasm_bindgen_futures::spawn_local(async move {
                let _ = JsFuture::from(promise).await;
            });
        }
    }
}
fn unavailable() -> HttpError {
    HttpError::new("object provider transport failed", true)
}
fn invalid() -> HttpError {
    HttpError::new("invalid object provider transport data", false)
}
#[async_trait]
impl HttpTransport for FetchTransport {
    async fn execute(&self, outgoing: HttpRequest) -> Result<HttpResponse, HttpError> {
        AssertSend(self.execute_inner(outgoing)).await
    }
}

impl FetchTransport {
    async fn execute_inner(&self, outgoing: HttpRequest) -> Result<HttpResponse, HttpError> {
        let callback = self.0.0.clone();
        // THREAT[TM-DURABLE-012]: the callback owns the configured endpoint
        // and credential; provider requests cannot select another host.
        if outgoing.method().as_str() != "POST"
            || outgoing.uri() != "https://isolate.invalid/v1/chat/completions"
        {
            return Err(invalid());
        }
        let headers = outgoing
            .headers()
            .iter()
            .map(|(name, value)| Ok((name.as_str(), value.to_str().map_err(|_| invalid())?)))
            .collect::<Result<Vec<_>, HttpError>>()?;
        let encoded = json!({"method":"POST","path":"/chat/completions","headers":headers,"body":std::str::from_utf8(outgoing.body()).map_err(|_| invalid())?}).to_string();
        if encoded.len() > 1024 * 1024 {
            return Err(invalid());
        }
        let mut guard = FetchGuard {
            controller: web_sys::AbortController::new().map_err(|_| unavailable())?,
            reader: None,
        };
        let pending = callback
            .call2(
                &JsValue::UNDEFINED,
                &JsValue::from_str(&encoded),
                guard.controller.signal().as_ref(),
            )
            .map_err(|_| unavailable())?;
        let response = everruns_contracts::rt::timeout(
            std::time::Duration::from_secs(60),
            JsFuture::from(js_sys::Promise::resolve(&pending)),
        )
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?;
        let response: web_sys::Response = response.dyn_into().map_err(|_| invalid())?;
        let status = StatusCode::from_u16(response.status()).map_err(|_| invalid())?;
        let mut headers = HeaderMap::new();
        for pair in js_sys::Array::from(response.headers().as_ref()).iter() {
            let pair = js_sys::Array::from(&pair);
            let name = pair.get(0).as_string().ok_or_else(invalid)?;
            let value = pair.get(1).as_string().ok_or_else(invalid)?;
            headers.append(
                header::HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid())?,
                header::HeaderValue::from_str(&value).map_err(|_| invalid())?,
            );
        }
        let Some(body) = response.body() else {
            return Ok(HttpResponse::new(
                status,
                headers,
                Box::pin(futures::stream::empty()),
            ));
        };
        guard.reader = Some(body.get_reader().dyn_into().map_err(|_| invalid())?);
        let body = futures::stream::unfold(Some((AssertSend(guard), 0usize)), |state| async move {
            let (guard, bytes) = state?;
            let read = AssertSend(async {
                JsFuture::from(guard.0.reader.as_ref().ok_or_else(invalid)?.read())
                    .await
                    .map_err(|_| unavailable())
            })
            .await;
            let result = match read {
                Ok(read) => read,
                Err(error) => return Some((Err(error), None)),
            };
            if Reflect::get(&result, &"done".into())
                .ok()
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return None;
            }
            let chunk = match Reflect::get(&result, &"value".into())
                .ok()
                .and_then(|v| v.dyn_into::<Uint8Array>().ok())
            {
                Some(chunk) => chunk,
                None => return Some((Err(invalid()), None)),
            };
            let total = bytes.saturating_add(chunk.byte_length() as usize);
            if total > 3 * 1024 * 1024 {
                return Some((
                    Err(HttpError::new("provider reply exceeds limit", false)),
                    None,
                ));
            }
            Some((Ok(chunk.to_vec().into()), Some((guard, total))))
        });
        Ok(HttpResponse::new(status, headers, Box::pin(body)))
    }
}
