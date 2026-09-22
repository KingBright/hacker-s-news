//! One edition, one publication and one multi-host audio job.
use super::*;

pub(super) fn validate(job: &AgentJob, artifact: &Value) -> Result<(), String> {
    for (field, max) in [("opening", 400), ("closing", 200)] {
        let text = trim_required(artifact[field].as_str().unwrap_or(""), field)?;
        if text.chars().count() > max || text.contains("http") || text.contains('<') {
            return Err(format!("invalid program {field}"));
        }
    }
    let sections = artifact["sections"]
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 30)
        .ok_or("program requires 1–30 category sections")?;
    let mut categories = std::collections::HashSet::new();
    let mut section_job = job.clone();
    section_job.job_type = "radio_episode".into();
    section_job.context_json = Some(json!({"production_mode":"external_agent"}).to_string());
    for section in sections {
        let category = trim_required(
            section["category"].as_str().unwrap_or(""),
            "section.category",
        )?;
        if !categories.insert(category) {
            return Err("program contains duplicate categories".into());
        }
        validate_external_artifact(&section_job, section)?;
    }
    let context = parse_json(job.context_json.as_deref());
    chrono::NaiveDate::parse_from_str(context["run_date"].as_str().unwrap_or(""), "%Y-%m-%d")
        .map_err(|_| "program requires valid context.run_date")?;
    if !matches!(
        context["slot"].as_str(),
        Some("morning-program" | "evening-program")
    ) {
        return Err("program slot must be morning-program or evening-program".into());
    }
    Ok(())
}

pub(super) async fn publish(
    conn: &mut sqlx::SqliteConnection,
    job: &AgentJob,
    artifact: &Value,
    priority: i64,
) -> Result<PublishResult, String> {
    validate(job, artifact)?;
    let context = parse_json(job.context_json.as_deref());
    let date = context["run_date"].as_str().unwrap();
    let slot = context["slot"].as_str().unwrap();
    let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_jobs WHERE job_type='radio_program' AND status='completed' AND json_extract(context_json,'$.run_date')=? AND json_extract(context_json,'$.slot')=?")
        .bind(date).bind(slot).fetch_one(&mut *conn).await.map_err(|e|e.to_string())?;
    if existing > 0 {
        return Err("this edition already has a published program; reuse its original job".into());
    }
    let sections = artifact["sections"].as_array().unwrap();
    let opening = artifact["opening"].as_str().unwrap();
    let closing = artifact["closing"].as_str().unwrap();
    let mut display = vec![opening.to_string()];
    let mut segments = vec![json!({"category":null,"text":opening})];
    let mut sources = vec![];
    for section in sections {
        display.push(format!(
            "【{}】\n{}",
            section["category"].as_str().unwrap(),
            section["script"].as_str().unwrap()
        ));
        segments.push(json!({"category":section["category"],"text":section["audio_script"].as_str().unwrap_or(section["script"].as_str().unwrap())}));
        sources.extend(
            section["sources"]
                .as_array()
                .ok_or("section sources missing")?
                .clone(),
        );
    }
    display.push(closing.to_string());
    segments.push(json!({"category":null,"text":closing}));
    let edition = slot.split('-').next().unwrap();
    let published = publish_radio_episode(conn, &json!({
        "title":artifact["title"],"category":"完整节目","script":display.join("\n\n"),"sources":[],
        "tags":["radio:program",format!("radio:date:{date}"),format!("radio:edition:{edition}"),format!("radio:sections:{}",sections.len())]
    }), priority).await?;
    // A program is a compilation, not a duplicate of its first source article.
    let item_id = published
        .result_ref
        .as_deref()
        .unwrap()
        .strip_prefix("radio_item:")
        .unwrap();
    insert_item_sources(
        conn,
        item_id,
        serde_json::from_value(json!(sources)).map_err(|e| e.to_string())?,
    )
    .await?;
    sqlx::query("UPDATE voice_jobs SET voice_kind='radio_program',text=? WHERE id=?")
        .bind(json!({"segments":segments}).to_string())
        .bind(&published.voice_job_ids[0])
        .execute(conn)
        .await
        .map_err(|e| e.to_string())?;
    Ok(published)
}
