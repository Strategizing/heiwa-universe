use spacetimedb::{table, reducer, view, Identity, ReducerContext, ViewContext, Table};

// 1. Private Table with Cryptographic Identity
#[table(name = organization_tasks, public = false)]
pub struct OrganizationTask {
    #[primarykey]
    #[autoinc]
    pub id: u64,
    pub tenant_id: Identity, // The cryptographic boundary
    pub payload: String,
    pub status: String,
    pub assigned_worker: Option<String>,
}

// 2. Discord Identity & Context Tables
#[table(name = discord_users, public = true)]
pub struct DiscordUser {
    #[primarykey]
    pub user_id: u64,
    pub username: String,
    pub trust_score: f32, // 0.0 to 1.0 (reputation)
    pub last_seen_at: u64, // Unix timestamp
}

#[table(name = discord_channels, public = true)]
pub struct DiscordChannel {
    #[primarykey]
    pub channel_id: u64,
    pub name: String,
    pub purpose: String, // e.g. "ingress", "telemetry", "brainstorm"
    pub metadata_json: String,
}

#[table(name = discord_interactions, public = true)]
pub struct DiscordInteraction {
    #[primarykey]
    #[autoinc]
    pub id: u64,
    pub user_id: u64,
    pub channel_id: u64,
    pub intent_class: String,
    pub timestamp: u64,
}

// 3. View: Restricts row synchronization to the authenticated client/limb
#[view(name = tenant_task_view, public = true)]
pub fn tenant_tasks(ctx: &ViewContext) -> Vec<OrganizationTask> {
    ctx.db.organization_tasks()
       .tenant_id()
       .filter(ctx.sender()) 
       .collect()
}

// 4. Reducers: State Mutations
#[reducer]
pub fn claim_task(ctx: &ReducerContext, task_id: u64, worker_id: String) -> Result<(), String> {
    let mut task = ctx.db.organization_tasks().id().find(task_id).ok_or("Task not found")?;
        
    if task.status != "pending" || task.assigned_worker.is_some() {
        return Err("Task already claimed by another Limb.".into());
    }
    
    task.status = "claimed".to_string();
    task.assigned_worker = Some(worker_id);
    ctx.db.organization_tasks().id().update(task_id, task);
    
    Ok(())
}

#[reducer]
pub fn upsert_discord_user(ctx: &ReducerContext, user_id: u64, username: String, trust_score: f32) -> Result<(), String> {
    let now = ctx.timestamp.unix_seconds();
    let existing = ctx.db.discord_users().user_id().find(user_id);
    
    if let Some(mut user) = existing {
        user.username = username;
        user.trust_score = trust_score;
        user.last_seen_at = now;
        ctx.db.discord_users().user_id().update(user_id, user);
    } else {
        ctx.db.discord_users().insert(DiscordUser {
            user_id,
            username,
            trust_score,
            last_seen_at: now,
        });
    }
    Ok(())
}

#[reducer]
pub fn register_discord_channel(ctx: &ReducerContext, channel_id: u64, name: String, purpose: String, metadata: String) -> Result<(), String> {
    let existing = ctx.db.discord_channels().channel_id().find(channel_id);
    
    if let Some(mut channel) = existing {
        channel.name = name;
        channel.purpose = purpose;
        channel.metadata_json = metadata;
        ctx.db.discord_channels().channel_id().update(channel_id, channel);
    } else {
        ctx.db.discord_channels().insert(DiscordChannel {
            channel_id,
            name,
            purpose,
            metadata_json: metadata,
        });
    }
    Ok(())
}

#[reducer]
pub fn record_interaction(ctx: &ReducerContext, user_id: u64, channel_id: u64, intent: String) -> Result<(), String> {
    ctx.db.discord_interactions().insert(DiscordInteraction {
        id: 0, // auto-inc
        user_id,
        channel_id,
        intent_class: intent,
        timestamp: ctx.timestamp.unix_seconds(),
    });
    Ok(())
}
