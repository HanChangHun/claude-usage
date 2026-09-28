use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Clone)]
pub struct Account {
    email: Option<String>,
    plan: Option<String>,
}

#[derive(Deserialize)]
struct Profile {
    email_address: Option<String>,
    #[serde(default)]
    memberships: Vec<Membership>,
}

#[derive(Deserialize)]
struct Membership {
    organization: Organization,
}

#[derive(Deserialize)]
struct Organization {
    uuid: String,
    plan_display_name: Option<String>,
    analytics_subscription_plan: Option<String>,
}

pub async fn read(client: &reqwest::Client, cookies: &str, org_id: &str) -> Option<Account> {
    let profile = client
        .get("https://claude.ai/api/account")
        .header("Cookie", cookies)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<Profile>()
        .await
        .ok()?;
    let plan = profile
        .memberships
        .into_iter()
        .find(|membership| membership.organization.uuid == org_id)
        .and_then(|membership| {
            let org = membership.organization;
            org.plan_display_name.or(org.analytics_subscription_plan)
        });
    Some(Account {
        email: profile.email_address,
        plan,
    })
}
