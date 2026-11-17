//! 按协议拉取厂商可用模型列表（`GET /models`）。
//!
//! 三套协议各自返回不同形状，统一解析成模型 id 数组：
//! - OpenAI 兼容：`{"data":[{"id":"gpt-4o"}, …]}`
//! - Anthropic：`{"data":[{"id":"claude-…"}, …]}`（带 `x-api-key` 头）
//! - Gemini：`{"models":[{"name":"models/gemini-2.5-pro"}, …]}`（`name` 要去前缀）
//!
//! 这是 UI 里「拉取模型」按钮的唯一实现：网络请求必须在后台线程跑，
//! 结果经 flume 送回 UI（阻塞调用放主线程会卡住渲染）。

use std::time::Duration;

use serde_json::Value;

use crate::ProviderConfig;

/// 按协议拉取模型列表。
///
/// `protocol` 取值与 [`crate::build_provider`] 一致：`"anthropic"` / `"gemini"` /
/// 其余按 OpenAI 兼容处理。返回按字典序排序、去重后的模型 id。
pub fn list_models(protocol: &str, config: &ProviderConfig) -> Result<Vec<String>, String> {
    let base = config.base_url.trim_end_matches('/');
    if base.is_empty() {
        return Err("先填写 API 基址".into());
    }
    if protocol != "gemini" && config.api_key.trim().is_empty() {
        return Err("先填写 API Key".into());
    }
    let agent = build_agent();
    let host = host_of(base);

    let result = match protocol {
        "anthropic" => agent
            .get(&format!("{base}/models?limit=1000"))
            .header("x-api-key", &config.api_key)
            .header("anthropic-version", "2023-06-01")
            .call(),
        "gemini" => agent
            .get(&format!(
                "{base}/models?pageSize=1000&key={}",
                config.api_key
            ))
            .call(),
        _ => agent
            .get(&format!("{base}/models"))
            .header("Authorization", &format!("Bearer {}", config.api_key))
            .call(),
    };

    let mut resp = result.map_err(|e| http_error(&host, e))?;
    let json: Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("解析 {host} 的响应失败：{e}"))?;

    let mut ids = match protocol {
        "anthropic" => extract_data_ids(&json),
        "gemini" => extract_gemini(&json),
        _ => extract_data_ids(&json),
    };
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Err(format!("{host} 返回了空的模型列表"));
    }
    Ok(ids)
}

fn build_agent() -> ureq::Agent {
    let mut b = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent("neverwork/0.1");
    // 尊重系统代理（ureq 默认不读环境变量，必须显式尝试）
    if let Some(p) = ureq::Proxy::try_from_env() {
        b = b.proxy(Some(p));
    }
    b.build().new_agent()
}

/// `{"data":[{"id":"…"}]}`，OpenAI 兼容与 Anthropic 共用。
fn extract_data_ids(json: &Value) -> Vec<String> {
    json.get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|v| v.as_str()))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// `{"models":[{"name":"models/gemini-2.5-pro"}]}`，`name` 去掉 `models/` 前缀。
fn extract_gemini(json: &Value) -> Vec<String> {
    json.get("models")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("name").and_then(|v| v.as_str()))
                .map(|s| s.trim().trim_start_matches("models/").to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn host_of(url: &str) -> String {
    url.split("//")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
}

fn http_error(host: &str, e: ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(code) => match code {
            401 | 403 => format!("{host} 拒绝访问（HTTP {code}，Key 可能无效或没有权限）"),
            404 => format!("{host} 没有 /models 接口（HTTP 404），请手动添加模型"),
            429 => format!("{host} 请求过于频繁（HTTP 429），稍后再试"),
            _ => format!("{host} 返回 HTTP {code}"),
        },
        other => format!("连接 {host} 失败：{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_style_ids() {
        let json: Value = serde_json::from_str(
            r#"{"object":"list","data":[{"id":"gpt-4o"},{"id":"o3"},{"id":"gpt-4o"}]}"#,
        )
        .unwrap();
        let ids = extract_data_ids(&json);
        assert_eq!(ids, vec!["gpt-4o", "o3", "gpt-4o"]);
    }

    #[test]
    fn anthropic_style_ids() {
        let json: Value = serde_json::from_str(
            r#"{"data":[{"id":"claude-sonnet-4-5","display_name":"Claude Sonnet 4.5"}],"has_more":false}"#,
        )
        .unwrap();
        assert_eq!(extract_data_ids(&json), vec!["claude-sonnet-4-5"]);
    }

    #[test]
    fn gemini_strips_models_prefix() {
        let json: Value = serde_json::from_str(
            r#"{"models":[{"name":"models/gemini-2.5-pro"},{"name":"gemini-2.0-flash"}]}"#,
        )
        .unwrap();
        assert_eq!(extract_gemini(&json), vec!["gemini-2.5-pro", "gemini-2.0-flash"]);
    }

    #[test]
    fn unknown_shape_yields_empty() {
        let json: Value = serde_json::from_str(r#"{"error":"nope"}"#).unwrap();
        assert!(extract_data_ids(&json).is_empty());
        assert!(extract_gemini(&json).is_empty());
    }

    #[test]
    fn list_models_rejects_empty_base_url() {
        let cfg = ProviderConfig {
            api_key: "k".into(),
            base_url: "  ".into(),
            ..ProviderConfig::default()
        };
        assert!(list_models("openai", &cfg).is_err());
    }

    #[test]
    fn host_of_strips_scheme() {
        assert_eq!(host_of("https://api.openai.com/v1"), "api.openai.com");
        assert_eq!(host_of("http://localhost:11434/v1"), "localhost:11434");
    }
}
