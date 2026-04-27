use crate::scorer::HealthScore;
use anyhow::Result;
use std::fs;

pub fn generate_html_report(scores: &[HealthScore], output_path: &str) -> Result<()> {
    let html = build_html(scores);
    fs::write(output_path, html)?;
    Ok(())
}

fn score_color(score: f64) -> &'static str {
    if score >= 80.0 {
        "#22c55e"
    } else if score >= 60.0 {
        "#eab308"
    } else {
        "#ef4444"
    }
}

fn build_html(scores: &[HealthScore]) -> String {
    // Collect all metric names
    let mut metric_names: Vec<String> = Vec::new();
    for hs in scores {
        for m in &hs.metrics {
            if !metric_names.contains(&m.name) {
                metric_names.push(m.name.clone());
            }
        }
    }

    // Labels (commit SHAs or timestamps)
    let labels: Vec<String> = scores
        .iter()
        .map(|hs| {
            hs.commit
                .as_deref()
                .map(|c| {
                    if c.len() > 8 {
                        c[..8].to_string()
                    } else {
                        c.to_string()
                    }
                })
                .unwrap_or_else(|| hs.timestamp.format("%Y-%m-%d").to_string())
        })
        .collect();

    let labels_json = serde_json::to_string(&labels).unwrap_or_default();

    // Overall scores dataset
    let overall_data: Vec<f64> = scores
        .iter()
        .map(|hs| (hs.overall * 10.0).round() / 10.0)
        .collect();
    // Metric colors
    let colors = [
        "#3b82f6", "#f97316", "#a855f7", "#14b8a6", "#f43f5e", "#84cc16", "#06b6d4", "#8b5cf6",
    ];

    // Build metric datasets safely via JSON serialization
    let mut metric_datasets = Vec::new();
    for (i, name) in metric_names.iter().enumerate() {
        let color = colors[i % colors.len()];
        let data: Vec<f64> = scores
            .iter()
            .map(|hs| {
                hs.metrics
                    .iter()
                    .find(|m| &m.name == name)
                    .map(|m| (m.score * 10.0).round() / 10.0)
                    .unwrap_or(0.0)
            })
            .collect();
        metric_datasets.push(serde_json::json!({
            "label": name,
            "data": data,
            "borderColor": color,
            "backgroundColor": format!("{color}22"),
            "borderWidth": 2,
            "tension": 0.3,
            "pointRadius": 4
        }));
    }
    let datasets_json = serde_json::to_string(&{
        let mut datasets = vec![serde_json::json!({
            "label": "Overall",
            "data": overall_data,
            "borderColor": "#38bdf8",
            "backgroundColor": "#38bdf822",
            "borderWidth": 3,
            "tension": 0.3,
            "pointRadius": 5
        })];
        datasets.extend(metric_datasets);
        datasets
    })
    .unwrap_or_default();

    // Build table rows
    let mut table_rows = String::new();
    for hs in scores {
        let commit_label = hs
            .commit
            .as_deref()
            .map(|c| if c.len() > 12 { &c[..12] } else { c })
            .unwrap_or("-");
        let date_label = hs.timestamp.format("%Y-%m-%d %H:%M").to_string();
        let overall_color = score_color(hs.overall);
        let metric_cells: String = metric_names
            .iter()
            .map(|name| {
                let m = hs.metrics.iter().find(|m| &m.name == name);
                match m {
                    Some(m) => {
                        let c = score_color(m.score);
                        format!(
                            r#"<td style="color:{}">{:.1}<br><small style="color:#888">{}</small></td>"#,
                            c,
                            m.score,
                            htmlize::escape_text(&m.details)
                        )
                    }
                    None => "<td>-</td>".to_string(),
                }
            })
            .collect();
        table_rows.push_str(&format!(
            r#"<tr>
                <td>{}</td>
                <td>{}</td>
                <td style="color:{};font-weight:bold">{:.1}</td>
                {}
            </tr>"#,
            htmlize::escape_text(commit_label),
            htmlize::escape_text(&date_label),
            overall_color,
            hs.overall,
            metric_cells
        ));
    }

    let metric_headers: String = metric_names
        .iter()
        .map(|n| format!("<th>{}</th>", htmlize::escape_text(n)))
        .collect();

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Fiber Health Score Report</title>
<!-- Pin to a specific version; verify SRI hash before deploying to production -->
<script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.7/dist/chart.umd.min.js"></script>
<style>
  body {{ font-family: system-ui, sans-serif; background: #0f172a; color: #e2e8f0; margin: 0; padding: 2rem; }}
  h1 {{ color: #38bdf8; margin-bottom: 0.5rem; }}
  .subtitle {{ color: #94a3b8; margin-bottom: 2rem; }}
  .chart-container {{ background: #1e293b; border-radius: 12px; padding: 1.5rem; margin-bottom: 2rem; max-width: 1200px; }}
  table {{ width: 100%; border-collapse: collapse; background: #1e293b; border-radius: 12px; overflow: hidden; max-width: 1200px; }}
  th {{ background: #334155; padding: 0.75rem 1rem; text-align: left; color: #94a3b8; font-size: 0.85rem; text-transform: uppercase; }}
  td {{ padding: 0.75rem 1rem; border-top: 1px solid #334155; font-size: 0.9rem; vertical-align: top; }}
  tr:hover td {{ background: #263244; }}
  small {{ font-size: 0.75rem; }}
</style>
</head>
<body>
<h1>🧵 Fiber Health Score Report</h1>
<p class="subtitle">Generated {}</p>
<div class="chart-container">
  <canvas id="chart"></canvas>
</div>
<table>
  <thead><tr><th>Commit</th><th>Date</th><th>Overall</th>{}</tr></thead>
  <tbody>{}</tbody>
</table>
<script>
const ctx = document.getElementById('chart').getContext('2d');
new Chart(ctx, {{
  type: 'line',
  data: {{
    labels: {},
    datasets: {}
  }},
  options: {{
    responsive: true,
    plugins: {{
      legend: {{ labels: {{ color: '#e2e8f0' }} }},
      tooltip: {{ mode: 'index', intersect: false }}
    }},
    scales: {{
      x: {{ ticks: {{ color: '#94a3b8' }}, grid: {{ color: '#334155' }} }},
      y: {{
        min: 0, max: 100,
        ticks: {{ color: '#94a3b8' }},
        grid: {{ color: '#334155' }}
      }}
    }}
  }}
}});
</script>
</body>
</html>"#,
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
        metric_headers,
        table_rows,
        labels_json,
        datasets_json,
    )
}
