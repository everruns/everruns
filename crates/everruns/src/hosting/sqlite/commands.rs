//! Storage-only adapter: shared semantics run under the IMMEDIATE transaction.
use super::*;
use crate::hosting::spi::session_commands::{
    self, CommandReceipt, CommandView, ControlUpdate, SessionCommand,
};

impl SqliteHosting {
    pub(super) fn execute_command(
        &self,
        session: SessionId,
        command: SessionCommand,
    ) -> Result<Value> {
        self.transaction(session,|tx,state| {
            let previous=if let Some(key)=command.key() {
                tx.query_row("SELECT fingerprint,value FROM framework_commands WHERE session=? AND id=?",params![session.to_string(),key],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).optional().map_err(error)?.map(|(fingerprint,value)| -> Result<CommandReceipt> {Ok(CommandReceipt{fingerprint:decode(fingerprint)?,value:decode(value)?})}).transpose()?
            } else {None};
            let input=if let SessionCommand::Cancel{input,..}=&command {Some(*input)}else{None};
            let assignment=input.map(|input|object::<super::super::TurnAssignment>(tx,session,"framework",&format!("assignment/{input}"))).transpose()?.flatten().map(|a|a.turn_id);
            let view=CommandView {
                session,now_ms:now() as i64,binding:state.binding.as_ref().map(|b|json!(b)),head:state.head,
                execution_active:state.holder.is_some() && state.expires>now(),suspended:state.suspended,
                suspension_reason:object::<Value>(tx,session,"framework","suspension")?.and_then(|v|v["reason"].as_str().map(str::to_owned)),
                open:state.open.map(|(turn,input)|EventContext::turn(turn,input)),pending:state.pending.clone(),
                configuration:object(tx,session,"framework","configuration")?.unwrap_or_default(),previous,
                command_count:tx.query_row("SELECT count(*) FROM framework_commands WHERE session=?",[session.to_string()],|r|r.get::<_,i64>(0)).map_err(error)? as usize,
                queued:if matches!(command,SessionCommand::Admit{..}) {tx.query_row("SELECT count(*) FROM (SELECT 1 FROM framework_events WHERE session=? AND type='input.message' AND turn_id IS NULL AND seq>? LIMIT 256)",params![session.to_string(),state.consumed],|r|r.get::<_,i64>(0)).map_err(error)? as usize}else{0},
                has_history:if matches!(command,SessionCommand::Seed{..}) {tx.query_row("SELECT EXISTS(SELECT 1 FROM framework_events WHERE session=? AND type IN ('input.message','output.message.completed'))",[session.to_string()],|r|r.get(0)).map_err(error)?}else{false},
                admitted:if let Some(input)=input {tx.query_row("SELECT EXISTS(SELECT 1 FROM framework_commands WHERE session=? AND id=?)",params![session.to_string(),input.to_string()],|r|r.get(0)).map_err(error)?}else{false},
                assignment,
                terminal:if let Some(turn)=assignment {tx.query_row("SELECT EXISTS(SELECT 1 FROM framework_events WHERE session=? AND identity=?)",params![session.to_string(),format!("terminal:{turn}")],|r|r.get(0)).map_err(error)?}else{false},
                rejected:input.map(|input|object::<Value>(tx,session,"framework",&format!("rejected/{input}"))).transpose()?.flatten().is_some(),
            };
            let plan=session_commands::prepare(command,view)?;
            if plan.replay {return Ok(plan.reply);}
            for object in plan.objects {put(tx,session,object.kind,&object.key,&object.value)?;}
            // Creation is the only operation whose binding does not yet exist.
            if let ControlUpdate::Bind{binding}=&plan.control {
                if state.binding.is_none(){state.created_at=everruns_contracts::rt::utc_now_iso();}
                state.binding=Some(serde_json::from_value(binding.clone()).map_err(error)?);
            }
            for event in plan.events {append(tx,state,event,None)?;}
            match plan.control {
                ControlUpdate::None|ControlUpdate::Bind{..}=>{},
                ControlUpdate::Admit{message,cancelled}=>{
                    if cancelled{state.target_cancels.insert(message,i64::from(state.head));}
                    state.inbox=i64::from(state.head);if state.next_input<=state.consumed{state.next_input=state.inbox;}state.runnable=true;
                },
                ControlUpdate::Resume=>{state.inbox=i64::from(state.head);state.resume_generation+=1;state.runnable=true;},
                ControlUpdate::RetryExecution=>{state.suspended=false;state.runnable=true;state.holder=None;state.expires=0;},
                ControlUpdate::Cancel{input,interrupt}=>{state.target_cancels.insert(input,state.inbox);if interrupt{state.cancel=Some(state.inbox);state.cancel_reported=false;state.runnable=true;}},
            }
            if let Some(key)=plan.key {retain_command(tx,session,&key,&plan.fingerprint,&plan.reply)?;}
            Ok(plan.reply)
        })
    }
}
