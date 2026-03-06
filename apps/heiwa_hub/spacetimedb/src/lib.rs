use spacetimedb::{table, reducer, view, Identity, ReducerContext, ViewContext};

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

// 2. View: Restricts row synchronization to the authenticated client/limb
#[view(name = tenant_task_view, public = true)]
pub fn tenant_tasks(ctx: &ViewContext) -> Vec<OrganizationTask> {
    ctx.db.organization_tasks()
       .tenant_id()
       .filter(ctx.sender()) 
       .collect()
}

// 3. Reducer: Atomic state mutation eliminating race conditions natively
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
