pub(crate) fn default_confession_stage(confession_stage: &str, pressure_level: &str) -> String {
    if pressure_level == "aggressive" && confession_stage == "denial" {
        return "vague".into();
    }
    confession_stage.to_string()
}

pub(crate) struct StageChangeReasonInput<'a> {
    pub(crate) current_stage: &'a str,
    pub(crate) reply_stage: &'a str,
    pub(crate) default_stage: &'a str,
    pub(crate) pressure_level: &'a str,
    pub(crate) reply_reason: Option<String>,
}

pub(crate) fn stage_change_reason(input: StageChangeReasonInput<'_>) -> Option<String> {
    if input.reply_stage == input.current_stage {
        return None;
    }

    input.reply_reason.or_else(|| {
        if input.reply_stage == input.default_stage
            && input.pressure_level == "aggressive"
            && input.current_stage == "denial"
        {
            Some("aggressive_pressure".into())
        } else {
            Some("agent_output".into())
        }
    })
}
