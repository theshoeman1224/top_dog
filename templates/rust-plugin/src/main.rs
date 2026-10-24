use top_dog_protocol::{ErrorCode, Operation, PluginError, Reply, Resolution};

fn handle(operation: Operation) -> Result<Reply, PluginError> {
    match operation {
        Operation::Describe=>Ok(Reply::Description {name:"template".into(),version:env!("CARGO_PKG_VERSION").into(),capabilities:vec!["diagnose".into(),"resolve".into()]}),
        Operation::Diagnose=>Ok(Reply::Diagnosis {available:true,message:"Template runs successfully. Replace this diagnosis with application-specific checks.".into()}),
        Operation::Resolve {process}=>{
            top_dog_core::monitor::validate(process).map_err(|e|PluginError::new(ErrorCode::StaleProcess,e))?;
            Ok(Reply::Resolution(Resolution {process,application:"Template".into(),summary:"Implement application-specific target discovery here.".into(),targets:vec![]}))
        }
        Operation::Activate {..}=>Err(PluginError::new(ErrorCode::Unsupported,"This template has no activation implementation")),
    }
}

fn main() {
    if let Err(error) =
        top_dog_protocol::serve(std::io::stdin().lock(), std::io::stdout().lock(), handle)
    {
        eprintln!("plugin protocol error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn template_never_claims_unsupported_activation() {
        let id = top_dog_core::monitor::identity(std::process::id() as i32).unwrap();
        assert!(
            matches!(handle(Operation::Resolve {process:id}).unwrap(),Reply::Resolution(r) if r.targets.is_empty())
        );
        assert!(matches!(
            handle(Operation::Activate {
                process: id,
                target_id: "anything".into()
            }),
            Err(PluginError {
                code: ErrorCode::Unsupported,
                ..
            })
        ));
    }
}
