use serve::prelude::*;

use crate::channels::slack::slack;

/// Every Monday at 09:00 UTC, post last week's revenue to #revenue.
#[schedule("0 9 * * MON")]
async fn weekly(cx: &Cx) -> Result {
    cx.start_session("Summarize last week's revenue")
        .deliver_to(slack::channel("C0123ABC"))
        .await
}
