use crate::metrics::MetricResult;
use crate::scorer::HealthScore;
use anyhow::Result;
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::fs;

pub fn generate_html_report(scores: &[HealthScore], output_path: &str) -> Result<()> {
    let html = build_html(scores)?;
    fs::write(output_path, html)?;
    Ok(())
}

fn json_for_html_script<T: serde::Serialize>(value: &T) -> Result<String> {
    // Prevent `</script>` from terminating script tags early in HTML.
    // Only allocate a second string when the escape sequence is actually present (#19).
    let json = serde_json::to_string(value)?;
    if json.contains("</") {
        Ok(json.replace("</", "<\\/"))
    } else {
        Ok(json)
    }
}

fn build_html(scores: &[HealthScore]) -> Result<String> {
    // Collect all metric names in insertion order, deduplicating with a HashSet.
    let mut metric_names: Vec<String> = Vec::new();
    let mut seen_metric_names: HashSet<&str> = HashSet::new();
    for hs in scores {
        for m in &hs.metrics {
            if seen_metric_names.insert(m.name.as_str()) {
                metric_names.push(m.name.clone());
            }
        }
    }

    // Build a metric_index for O(1) name→column lookup, and a dense matrix of
    // Option<&MetricResult> per (commit, metric) pair so iteration never re-hashes (#21).
    let metric_index: std::collections::HashMap<&str, usize> = metric_names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let score_matrix: Vec<Vec<Option<&MetricResult>>> = scores
        .iter()
        .map(|hs| {
            let mut row = vec![None; metric_names.len()];
            for m in &hs.metrics {
                if let Some(&idx) = metric_index.get(m.name.as_str()) {
                    row[idx] = Some(m);
                }
            }
            row
        })
        .collect();

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

    let labels_json = json_for_html_script(&labels)?;

    // Stacked bar chart: one dataset per metric, value = total_penalty for that commit.
    // Use a typed struct instead of serde_json::json! to avoid DOM allocation (#20).
    #[derive(serde::Serialize)]
    struct ChartDataset<'a> {
        label: &'a str,
        data: Vec<f64>,
        #[serde(rename = "backgroundColor")]
        background_color: &'static str,
        #[serde(rename = "borderColor")]
        border_color: &'static str,
        #[serde(rename = "borderWidth")]
        border_width: u32,
    }

    let colors: &[&'static str] = &[
        "#3b82f6", "#f97316", "#a855f7", "#14b8a6", "#f43f5e", "#84cc16", "#06b6d4", "#8b5cf6",
    ];

    let mut metric_datasets: Vec<ChartDataset<'_>> = Vec::new();
    for (i, name) in metric_names.iter().enumerate() {
        let color = colors[i % colors.len()];
        let data: Vec<f64> = score_matrix
            .iter()
            .map(|row| {
                row[i]
                    .map(|m| (m.total_penalty * 10.0).round() / 10.0)
                    .unwrap_or(0.0)
            })
            .collect();
        metric_datasets.push(ChartDataset {
            label: name.as_str(),
            data,
            background_color: color,
            border_color: color,
            border_width: 1,
        });
    }
    let datasets_json = json_for_html_script(&metric_datasets)?;

    // Build table rows using write! to avoid repeated small allocations (#18).
    // Pre-allocate a reasonable capacity based on the number of rows (#22).
    const ESTIMATED_HTML_BYTES_PER_ROW: usize = 120;
    const ESTIMATED_HTML_BYTES_PER_METRIC_CELL: usize = 60;
    let row_estimate = scores.len()
        * (ESTIMATED_HTML_BYTES_PER_ROW
            + metric_names.len() * ESTIMATED_HTML_BYTES_PER_METRIC_CELL);
    let mut table_rows = String::with_capacity(row_estimate);
    for (hs, row) in scores.iter().zip(score_matrix.iter()) {
        let commit_label = hs
            .commit
            .as_deref()
            .map(|c| if c.len() > 12 { &c[..12] } else { c })
            .unwrap_or("-");
        let date_label = hs.timestamp.format("%Y-%m-%d %H:%M").to_string();
        write!(
            table_rows,
            "<tr><td>{}</td><td>{}</td><td style=\"font-weight:bold\">{:.1}</td>",
            htmlize::escape_text(commit_label),
            htmlize::escape_text(&date_label),
            hs.overall
        )
        .unwrap();
        for cell in row {
            match cell {
                Some(m) => write!(
                    table_rows,
                    "<td>{:.1}<br><small style=\"color:#888\">{}</small></td>",
                    m.total_penalty,
                    htmlize::escape_text(&m.details)
                )
                .unwrap(),
                None => table_rows.push_str("<td>-</td>"),
            }
        }
        table_rows.push_str("</tr>");
    }

    let metric_headers: String = metric_names
        .iter()
        .map(|n| format!("<th>{}</th>", htmlize::escape_text(n)))
        .collect();

    // Pre-allocate the output buffer to avoid repeated reallocation (#22).
    // The fixed HTML skeleton (boilerplate, styles, JS chart setup) is roughly 2 KB;
    // add headroom for the metric_headers string and formatting overhead.
    const HTML_SKELETON_BYTES: usize = 4096;
    let html_estimate =
        HTML_SKELETON_BYTES + table_rows.len() + labels_json.len() + datasets_json.len();
    let mut html = String::with_capacity(html_estimate);
    write!(
        html,
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Fiber Penalty Report</title>
<!-- Pin to a specific version; verify SRI hash before deploying to production -->
<script src="https://cdnjs.cloudflare.com/ajax/libs/Chart.js/4.5.0/chart.min.js" integrity="sha512-n/G+dROKbKL3GVngGWmWfwK0yPctjZQM752diVYnXZtD/48agpUKLIn0xDQL9ydZ91x6BiOmTIFwWjjFi2kEFg==" crossorigin="anonymous" referrerpolicy="no-referrer"></script>
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
<h1>🧵 Fiber Penalty Report</h1>
<p class="subtitle">Generated {} &mdash; lower is better, 0 = perfect</p>
<div class="chart-container">
  <canvas id="chart"></canvas>
</div>
<table>
  <thead><tr><th>Commit</th><th>Date</th><th>Total Penalty</th>{}</tr></thead>
  <tbody>{}</tbody>
</table>
<script id="chart-labels" type="application/json">{}</script>
<script id="chart-datasets" type="application/json">{}</script>
<script>
const labels = JSON.parse(document.getElementById('chart-labels').textContent);
const datasets = JSON.parse(document.getElementById('chart-datasets').textContent);
const ctx = document.getElementById('chart').getContext('2d');
new Chart(ctx, {{
  type: 'bar',
  data: {{
    labels,
    datasets
  }},
  options: {{
    responsive: true,
    plugins: {{
      legend: {{ labels: {{ color: '#e2e8f0' }} }},
      tooltip: {{ mode: 'index', intersect: false }}
    }},
    scales: {{
      x: {{ stacked: true, ticks: {{ color: '#94a3b8' }}, grid: {{ color: '#334155' }} }},
      y: {{ stacked: true, ticks: {{ color: '#94a3b8' }}, grid: {{ color: '#334155' }} }}
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
    .unwrap();
    Ok(html)
}
