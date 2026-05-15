use serde::{de::DeserializeOwned, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

pub async fn invoke<A: Serialize, R: DeserializeOwned>(
    cmd: &str,
    args: &A,
) -> Result<R, String> {
    let args_js = serde_wasm_bindgen::to_value(args).map_err(|e| e.to_string())?;
    let result = tauri_invoke(cmd, args_js)
        .await
        .map_err(|e| {
            let msg: String = serde_wasm_bindgen::from_value(e).unwrap_or_else(|_| "Okänt fel".into());
            msg
        })?;
    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

pub async fn invoke_no_args<R: DeserializeOwned>(cmd: &str) -> Result<R, String> {
    let args = serde_json::json!({});
    invoke(cmd, &args).await
}
