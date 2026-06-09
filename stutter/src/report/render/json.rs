use serde::Serialize;

pub(crate) fn render_json_pretty<T>(value: &T) -> anyhow::Result<String>
where
    T: Serialize + ?Sized,
{
    Ok(serde_json::to_string_pretty(value)?)
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::render_json_pretty;

    #[derive(Serialize)]
    struct JsonRenderFixture {
        name: &'static str,
        count: u64,
    }

    #[test]
    fn json_renderer_returns_pretty_json_string() {
        let rendered = render_json_pretty(&JsonRenderFixture {
            name: "report",
            count: 2,
        })
        .unwrap();

        assert!(rendered.contains("\"name\": \"report\""));
        assert!(rendered.contains("\"count\": 2"));
        assert!(rendered.contains('\n'));
    }
}
