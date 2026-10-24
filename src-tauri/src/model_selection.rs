//! Shared validation for the model choices exposed by the local configuration editors.
pub fn models(default: &str, selected: &[String]) -> Result<Vec<String>, String> {
    if selected.len() > 200 {
        return Err("最多配置 200 个模型".into());
    }
    let mut models = Vec::new();
    for raw in selected.iter().map(String::as_str).chain([default]) {
        let model = raw.trim();
        if raw.len() > 200
            || raw.chars().any(char::is_control)
            || (!raw.is_empty() && model.is_empty())
        {
            return Err("请填写有效的模型 ID".into());
        }
        if !model.is_empty() && !models.iter().any(|value| value == model) {
            models.push(model.to_string());
        }
    }
    if models.len() > 200 {
        return Err("最多配置 200 个模型".into());
    }
    Ok(models)
}

pub fn effort(value: &str, supported: &[&str]) -> Result<(), String> {
    if !supported.contains(&value) {
        return Err("不支持的思考等级".into());
    }
    Ok(())
}
