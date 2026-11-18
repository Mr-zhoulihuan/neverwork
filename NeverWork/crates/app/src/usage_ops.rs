//! 「用量」弹框的行为：本会话 / 本工作区的 token 汇总 + 当前厂商余额查询。
//!
//! 余额只走厂商**公开的官方接口**（DeepSeek / Moonshot / SiliconFlow /
//! OpenRouter），其余厂商没有余额接口，面板里如实写「不支持」，不去猜。
//! 用量取自本地会话消息里厂商回传的 `tokens_in/tokens_out`，不额外发请求。

use std::collections::BTreeMap;
use std::time::Duration;

use gpui::Context;
use neverwork_core::{MessageKind, ProviderPreset};
use serde_json::Value;

use crate::state::AppState;

/// 余额查询结果。
#[derive(Debug, Clone)]
pub(crate) enum BalanceOutcome {
    /// 该厂商不提供公开的余额查询接口。
    Unsupported,
    Ok(BalanceInfo),
    Failed(String),
}

/// 余额卡片的数据。
#[derive(Debug, Clone, Default)]
pub(crate) struct BalanceInfo {
    /// 币种代码（CNY / USD）。
    pub currency: String,
    /// 总余额（保留厂商原始精度）。
    pub total: String,
    /// 附注行（充值 / 赠金 / 已用额度等）。
    pub rows: Vec<(String, String)>,
    /// 厂商返回的可读备注（如「账户不可用」）。
    pub note: String,
}

/// 单个模型的用量汇总。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelUsage {
    pub provider: String,
    pub model: String,
    pub requests: usize,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tools: usize,
}

/// 一个会话的用量合计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct UsageTotals {
    pub requests: usize,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub tools: usize,
    pub elapsed_ms: u64,
}

/// 币种符号；不认识的币种就原样显示代码。
pub(crate) fn currency_symbol(currency: &str) -> &'static str {
    match currency.trim().to_ascii_uppercase().as_str() {
        "CNY" => "¥",
        "USD" => "$",
        _ => "",
    }
}

/// 余额接口地址；返回 `None` 表示该厂商不提供余额查询。
pub(crate) fn balance_url(preset: ProviderPreset, base_url: &str) -> Option<String> {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    match preset {
        ProviderPreset::DeepSeek => {
            // DeepSeek 的余额接口在根路径，不在 /v1 下。
            let root = base.strip_suffix("/v1").unwrap_or(base);
            Some(format!("{root}/user/balance"))
        }
        ProviderPreset::Moonshot => Some(format!("{base}/users/me/balance")),
        ProviderPreset::SiliconFlow => Some(format!("{base}/user/info")),
        ProviderPreset::OpenRouter => Some(format!("{base}/credits")),
        _ => None,
    }
}

/// 同步查询余额（后台线程里跑；调用方负责不阻塞 UI）。
pub(crate) fn fetch_balance(
    preset: ProviderPreset,
    base_url: &str,
    api_key: &str,
) -> BalanceOutcome {
    let Some(url) = balance_url(preset, base_url) else {
        return BalanceOutcome::Unsupported;
    };
    let agent = {
        let mut builder = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(12)));
        if let Some(p) = ureq::Proxy::try_from_env() {
            builder = builder.proxy(Some(p));
        }
        builder.build().new_agent()
    };
    let resp = match agent
        .get(&url)
        .header("Authorization", &format!("Bearer {}", api_key.trim()))
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(code)) => {
            return BalanceOutcome::Failed(format!("HTTP {code}"));
        }
        Err(e) => return BalanceOutcome::Failed(format!("请求失败：{e}")),
    };
    let status = resp.status().as_u16();
    let mut body = resp.into_body();
    let raw = match body.with_config().limit(64 * 1024).read_to_string() {
        Ok(s) => s,
        Err(e) => return BalanceOutcome::Failed(format!("读取响应失败：{e}")),
    };
    if status >= 400 {
        return BalanceOutcome::Failed(format!("HTTP {status}{}", server_hint(&raw)));
    }
    match parse_balance(preset, &raw) {
        Some(info) => BalanceOutcome::Ok(info),
        None => BalanceOutcome::Failed("余额接口返回了无法识别的数据".into()),
    }
}

/// 服务端错误正文压成一行附在状态码后面（和 openai.rs 同一套口径）。
fn server_hint(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let msg = serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .or_else(|| v.get("message").and_then(|m| m.as_str()))
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| raw.to_string());
    let one = msg.replace(['\n', '\r'], " ");
    let clipped: String = one.trim().chars().take(200).collect();
    if clipped.is_empty() {
        String::new()
    } else {
        format!("：{clipped}")
    }
}

/// 解析各家余额接口的响应。字段名按各家官方文档，取不到关键字段就算失败。
pub(crate) fn parse_balance(preset: ProviderPreset, raw: &str) -> Option<BalanceInfo> {
    let v: Value = serde_json::from_str(raw).ok()?;
    match preset {
        ProviderPreset::DeepSeek => {
            let info = v.get("balance_infos")?.as_array()?.first()?;
            let currency = info
                .get("currency")
                .and_then(|c| c.as_str())
                .unwrap_or("CNY")
                .to_string();
            let total = value_str(info.get("total_balance")?)?;
            let mut rows = Vec::new();
            if let Some(s) = info.get("topped_up_balance").and_then(value_str) {
                rows.push(("充值余额".to_string(), s));
            }
            if let Some(s) = info.get("granted_balance").and_then(value_str) {
                rows.push(("赠送余额".to_string(), s));
            }
            let note = if v.get("is_available").and_then(|b| b.as_bool()) == Some(false) {
                "账户当前不可用（余额不足或欠费）".to_string()
            } else {
                String::new()
            };
            Some(BalanceInfo {
                currency,
                total,
                rows,
                note,
            })
        }
        ProviderPreset::Moonshot => {
            let data = v.get("data")?;
            let total = value_str(data.get("available_balance")?)?;
            let mut rows = Vec::new();
            if let Some(s) = data.get("cash_balance").and_then(value_str) {
                rows.push(("现金余额".to_string(), s));
            }
            if let Some(s) = data.get("voucher_balance").and_then(value_str) {
                rows.push(("代金券".to_string(), s));
            }
            Some(BalanceInfo {
                currency: "CNY".into(),
                total,
                rows,
                note: String::new(),
            })
        }
        ProviderPreset::SiliconFlow => {
            let data = v.get("data")?;
            let total = value_str(data.get("totalBalance").or_else(|| data.get("balance"))?)?;
            let mut rows = Vec::new();
            if let Some(s) = data.get("chargeBalance").and_then(value_str) {
                rows.push(("充值余额".to_string(), s));
            }
            if let Some(s) = data.get("balance").and_then(value_str) {
                rows.push(("余额".to_string(), s));
            }
            Some(BalanceInfo {
                currency: "CNY".into(),
                total,
                rows,
                note: String::new(),
            })
        }
        ProviderPreset::OpenRouter => {
            let data = v.get("data")?;
            let credits = number(data.get("total_credits")?)?;
            let usage = number(data.get("total_usage")?).unwrap_or(0.0);
            let left = (credits - usage).max(0.0);
            Some(BalanceInfo {
                currency: "USD".into(),
                total: format!("{left:.2}"),
                rows: vec![
                    ("已购额度".to_string(), format!("{credits:.2}")),
                    ("已用额度".to_string(), format!("{usage:.2}")),
                ],
                note: String::new(),
            })
        }
        _ => None,
    }
}

fn value_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

impl AppState {
    /// 打开「用量」弹框并查一次余额。
    pub(crate) fn open_usage(&mut self, cx: &mut Context<Self>) {
        self.usage_open = true;
        self.refresh_usage_balance(cx);
    }

    pub(crate) fn close_usage(&mut self, cx: &mut Context<Self>) {
        self.usage_open = false;
        cx.notify();
    }

    /// 查询当前会话厂商的余额（后台线程，结果经 flume 回 UI）。
    pub(crate) fn refresh_usage_balance(&mut self, cx: &mut Context<Self>) {
        if self.usage_balance_busy {
            return;
        }
        let Some(p) = self.provider_for_session(self.active_session) else {
            self.usage_balance = Some(BalanceOutcome::Failed(
                "还没有可用的厂商：先在「设置 → 模型」里添加并填好 Key".into(),
            ));
            self.usage_balance_pid.clear();
            cx.notify();
            return;
        };
        let preset = p.preset;
        let base_url = p.base_url.clone();
        let api_key = p.api_key.clone();
        let pid = p.id.clone();

        if balance_url(preset, &base_url).is_none() {
            // 没有公开接口的厂商直接定论，不发无意义的请求。
            self.usage_balance = Some(BalanceOutcome::Unsupported);
            self.usage_balance_pid = pid;
            cx.notify();
            return;
        }
        if api_key.trim().is_empty() {
            // 查余额只需要 Key，不需要选模型。
            self.usage_balance = Some(BalanceOutcome::Failed(
                "这个厂商还没有填 API Key，余额接口要先有 Key 才能查".into(),
            ));
            self.usage_balance_pid = pid;
            cx.notify();
            return;
        }

        self.usage_balance_busy = true;
        self.usage_balance_pid = pid.clone();
        self.usage_balance = None;
        cx.notify();

        let (tx, rx) = flume::bounded::<BalanceOutcome>(1);
        std::thread::spawn(move || {
            let _ = tx.send(fetch_balance(preset, &base_url, &api_key));
        });
        cx.spawn(async move |weak, async_cx| {
            if let Ok(out) = rx.recv_async().await {
                let _ = weak.update(async_cx, |this, cx| {
                    this.usage_balance_busy = false;
                    // 查询期间切到了别的厂商：结果已不对应当前会话，丢掉。
                    if this.usage_balance_pid == pid {
                        this.usage_balance = Some(out);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// 某个会话的用量合计。
    pub(crate) fn usage_totals(&self, session_idx: usize) -> UsageTotals {
        let mut t = UsageTotals::default();
        let Some(s) = self.sessions.get(session_idx) else {
            return t;
        };
        for m in &s.messages {
            match m.kind {
                MessageKind::Agent { .. } => {
                    t.requests += 1;
                    t.tokens_in += m.tokens_in;
                    t.tokens_out += m.tokens_out;
                    t.elapsed_ms += m.elapsed_ms;
                }
                MessageKind::ToolCall { .. } => t.tools += 1,
                _ => {}
            }
        }
        t
    }

    /// 本工作区按「厂商 · 模型」聚合的用量，消耗大的在前。
    pub(crate) fn usage_by_model(&self) -> Vec<ModelUsage> {
        let mut map: BTreeMap<(String, String), ModelUsage> = BTreeMap::new();
        for idx in 0..self.sessions.len() {
            let t = self.usage_totals(idx);
            if t.requests == 0 {
                continue;
            }
            let (provider, model) = self.usage_session_label(idx);
            let entry = map
                .entry((provider.clone(), model.clone()))
                .or_insert(ModelUsage {
                    provider,
                    model,
                    requests: 0,
                    tokens_in: 0,
                    tokens_out: 0,
                    tools: 0,
                });
            entry.requests += t.requests;
            entry.tokens_in += t.tokens_in;
            entry.tokens_out += t.tokens_out;
            entry.tools += t.tools;
        }
        let mut rows: Vec<ModelUsage> = map.into_values().collect();
        rows.sort_by(|a, b| {
            (b.tokens_in + b.tokens_out)
                .cmp(&(a.tokens_in + a.tokens_out))
                .then_with(|| b.requests.cmp(&a.requests))
        });
        rows
    }

    /// 汇总用的「厂商 · 模型」标签。
    ///
    /// 用会话**自己记录的** provider_id/model（空 = 跟随当时的默认），
    /// 不走 `provider_for_session` 的回落逻辑：回落会把旧会话的消耗算到
    /// 当前默认厂商头上，汇总表就失真了。
    fn usage_session_label(&self, idx: usize) -> (String, String) {
        let Some(s) = self.sessions.get(idx) else {
            return (String::new(), String::new());
        };
        let pid = s.provider_id.trim();
        let fallback_model = self
            .settings
            .providers
            .iter()
            .find(|p| p.id == pid)
            .map(|p| p.effective_model())
            .unwrap_or_default();
        let provider = if pid.is_empty() {
            match self.settings.active_provider() {
                Some(p) => format!("{}（默认）", p.name),
                None => "未配置厂商".to_string(),
            }
        } else {
            self.settings
                .providers
                .iter()
                .find(|p| p.id == pid)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| pid.to_string())
        };
        let model = if s.model.trim().is_empty() {
            fallback_model
        } else {
            s.model.clone()
        };
        (provider, model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_balance_uses_root_endpoint() {
        assert_eq!(
            balance_url(ProviderPreset::DeepSeek, "https://api.deepseek.com/v1/").as_deref(),
            Some("https://api.deepseek.com/user/balance")
        );
        // 用户自己填了不带 /v1 的根地址也要能查
        assert_eq!(
            balance_url(ProviderPreset::DeepSeek, "https://api.deepseek.com").as_deref(),
            Some("https://api.deepseek.com/user/balance")
        );
    }

    #[test]
    fn glm_has_no_balance_endpoint() {
        assert!(balance_url(ProviderPreset::Glm, "https://open.bigmodel.cn/api/paas/v4").is_none());
    }

    #[test]
    fn deepseek_balance_parsed() {
        let raw = r#"{"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"110.00","granted_balance":"10.00","topped_up_balance":"100.00"}]}"#;
        let b = parse_balance(ProviderPreset::DeepSeek, raw).unwrap();
        assert_eq!(b.total, "110.00");
        assert_eq!(b.currency, "CNY");
        assert_eq!(b.rows.len(), 2);
        assert!(b.note.is_empty());
    }

    #[test]
    fn openrouter_balance_is_credits_minus_usage() {
        let raw = r#"{"data":{"total_credits":100,"total_usage":20.5}}"#;
        let b = parse_balance(ProviderPreset::OpenRouter, raw).unwrap();
        assert_eq!(b.total, "79.50");
        assert_eq!(b.currency, "USD");
    }

    #[test]
    fn moonshot_and_siliconflow_parse() {
        let m = parse_balance(
            ProviderPreset::Moonshot,
            r#"{"code":0,"data":{"available_balance":12.5,"cash_balance":10,"voucher_balance":2.5}}"#,
        )
        .unwrap();
        assert_eq!(m.total, "12.5");

        let s = parse_balance(
            ProviderPreset::SiliconFlow,
            r#"{"code":20000,"data":{"balance":"1.20","chargeBalance":"1.20","totalBalance":"1.20"}}"#,
        )
        .unwrap();
        assert_eq!(s.total, "1.20");
    }

    #[test]
    fn unknown_shape_is_none() {
        assert!(parse_balance(ProviderPreset::DeepSeek, "{}").is_none());
        assert!(parse_balance(ProviderPreset::DeepSeek, "not json").is_none());
    }

    #[test]
    fn currency_symbol_maps_known_currencies() {
        assert_eq!(currency_symbol("CNY"), "¥");
        assert_eq!(currency_symbol("usd"), "$");
        assert_eq!(currency_symbol("EUR"), "");
    }
}
