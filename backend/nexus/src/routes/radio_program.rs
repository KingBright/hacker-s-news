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
    for field in ["script", "audio_script"] {
        let text = sections.iter().filter_map(|s| s[field].as_str().or(s["script"].as_str())).collect::<Vec<_>>().join("\n");
        validate_narration(&text)?;
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

// Guarded replacement: retain item/bookmarks/date, archive old evidence, queue one new voice job.
pub(super) async fn repair(conn: &mut sqlx::SqliteConnection, id: &str, request: &RepairRadioProgramRequest) -> Result<PublishResult, String> {
    sqlx::query("UPDATE agent_jobs SET updated_at=updated_at WHERE id=?").bind(id).execute(&mut *conn).await.map_err(|e| e.to_string())?;
    let job = sqlx::query_as::<_, AgentJob>("SELECT * FROM agent_jobs WHERE id=?").bind(id).fetch_one(&mut *conn).await.map_err(|e| e.to_string())?;
    if job.job_type != "radio_program" || job.status != "completed" { return Err("repair requires a completed radio_program".into()); }
    validate(&job, &request.artifact)?;
    let item_id = job.result_ref.as_deref().and_then(|s| s.strip_prefix("radio_item:")).ok_or("repair target missing")?;
    let old = sqlx::query_as::<_, VoiceJob>("SELECT * FROM voice_jobs WHERE id=? AND target_type='radio_item' AND target_id=? AND voice_kind='radio_program'").bind(&request.expected_voice_job_id).bind(item_id).fetch_one(&mut *conn).await.map_err(|_| "expected voice job does not belong to this program".to_string())?;
    if old.status == "cancelled" && parse_json(job.artifact_json.as_deref()) == request.artifact {
        let ids = sqlx::query_scalar::<_, String>("SELECT id FROM voice_jobs WHERE target_type='radio_item' AND target_id=? AND status!='cancelled'").bind(item_id).fetch_all(&mut *conn).await.map_err(|e| e.to_string())?;
        return Ok(PublishResult {result_ref: job.result_ref, voice_job_ids: ids});
    }
    if old.status != "completed" { return Err("repair expected voice job changed or is still active".into()); }
    let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM voice_jobs WHERE target_type='radio_item' AND target_id=? AND status IN ('queued','leased')").bind(item_id).fetch_one(&mut *conn).await.map_err(|e| e.to_string())?;
    if active != 0 { return Err("program already has an active voice job".into()); }
    let artifact = &request.artifact;
    let mut display = vec![artifact["opening"].as_str().unwrap().to_string()];
    let mut segments = vec![json!({"category":null,"text":artifact["opening"]})];
    let mut sources = vec![];
    for section in artifact["sections"].as_array().unwrap() {
        display.push(format!("【{}】\n{}", section["category"].as_str().unwrap(), section["script"].as_str().unwrap()));
        segments.push(json!({"category":section["category"],"text":section["audio_script"].as_str().unwrap_or(section["script"].as_str().unwrap())}));
        sources.extend(section["sources"].as_array().ok_or("section sources missing")?.clone());
    }
    display.push(artifact["closing"].as_str().unwrap().into());
    segments.push(json!({"category":null,"text":artifact["closing"]}));
    let summary = display.join("\n\n");
    validate_text_length(&summary, "script")?;
    let title = trim_required(artifact["title"].as_str().unwrap_or(""), "title")?;
    if title.len() > MAX_TITLE_LENGTH { return Err("title too long".into()); }
    let now = chrono::Utc::now().timestamp();
    let changed = sqlx::query("UPDATE items SET title=?,summary=?,audio_url=NULL,duration_sec=NULL WHERE id=? AND audio_url=?")
        .bind(title).bind(summary).bind(item_id).bind(&old.audio_url).execute(&mut *conn).await.map_err(|e| e.to_string())?;
    if changed.rows_affected()!=1 { return Err("published audio changed; inspect before repair".into()); }
    sqlx::query("DELETE FROM item_sources WHERE item_id=?").bind(item_id).execute(&mut *conn).await.map_err(|e| e.to_string())?;
    insert_item_sources(conn, item_id, serde_json::from_value(json!(sources)).map_err(|e| e.to_string())?).await?;
    sqlx::query("UPDATE voice_jobs SET status='cancelled',last_error='Superseded by reviewed manuscript repair',updated_at=? WHERE id=?")
        .bind(now).bind(&old.id).execute(&mut *conn).await.map_err(|e| e.to_string())?;
    let text = json!({"segments":segments}).to_string();
    let voice_id = insert_voice_job_on(conn, NewVoiceJob {target_type:"radio_item",target_id:item_id,product_line:"radio",voice_kind:"radio_program",text:&text,file_prefix:"radio_agent",priority:20}).await?;
    // Preserve the submitted artifact in the event journal before replacing it.
    sqlx::query("INSERT INTO agent_job_events (id,job_id,event_type,actor,payload_json,created_at) VALUES (?,?,'manuscript_repaired','maintenance',?,?)")
        .bind(Uuid::new_v4().to_string()).bind(id).bind(json!({"previous_artifact":parse_json(job.artifact_json.as_deref()),"previous_voice_job_id":old.id,"voice_job_id":voice_id}).to_string()).bind(now).execute(&mut *conn).await.map_err(|e| e.to_string())?;
    sqlx::query("UPDATE agent_jobs SET artifact_json=?,updated_at=? WHERE id=?").bind(artifact.to_string()).bind(now).bind(id).execute(&mut *conn).await.map_err(|e| e.to_string())?;
    Ok(PublishResult {result_ref:job.result_ref,voice_job_ids:vec![voice_id]})
}
