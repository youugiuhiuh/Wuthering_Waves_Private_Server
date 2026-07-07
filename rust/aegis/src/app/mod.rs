pub mod auth;
pub mod batch_handler;
pub mod destruct_flow;
pub mod state;

pub fn format_duration_human(secs: u64) -> String {
    if secs < 60 {
        format!("{}秒", secs)
    } else if secs < 3600 {
        format!("{}分钟", secs / 60)
    } else if secs < 86400 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m == 0 {
            format!("{}小时", h)
        } else {
            format!("{}小时{}分", h, m)
        }
    } else {
        let d = secs / 86400;
        let h = (secs % 86400) / 3600;
        let m = (secs % 3600) / 60;
        if h == 0 {
            format!("{}天", d)
        } else if m == 0 {
            format!("{}天{}小时", d, h)
        } else {
            format!("{}天{}小时{}分", d, h, m)
        }
    }
}
