//! Sys split dns; resource rules remain domain-owned.
use super::*;

pub(super) async fn assess_split_dns(
    host: &impl SplitDnsObservationHost,
    previous: Option<&SysSplitDnsStateV1>,
    desired: Option<&SysSplitDnsStateV1>,
) -> Result<SysRecoveryAssessment> {
    let previous_current = match previous {
        Some(state) => split_dns_matches(host, state).await?,
        None => false,
    };
    let desired_current = match desired {
        Some(state) => split_dns_matches(host, state).await?,
        None => false,
    };
    let resources_differ = previous.zip(desired).is_some_and(|(previous, desired)| {
        previous.resource != desired.resource || previous.os_id != desired.os_id
    });
    if resources_differ {
        let previous_exists = split_dns_exists(host, previous.expect("zipped above")).await?;
        let desired_exists = split_dns_exists(host, desired.expect("zipped above")).await?;
        return Ok(if previous_current && !desired_exists {
            SysRecoveryAssessment::Unchanged
        } else if !previous_exists && desired_current {
            SysRecoveryAssessment::Rollback
        } else {
            SysRecoveryAssessment::Blocked
        });
    }
    let state = previous.or(desired).context("split DNS transition state")?;
    let exists = split_dns_exists(host, state).await?;
    Ok(if previous.is_some() && previous_current {
        SysRecoveryAssessment::Unchanged
    } else if (desired.is_none() && !exists) || (desired.is_some() && desired_current) {
        SysRecoveryAssessment::Rollback
    } else if previous.is_none() && !exists {
        SysRecoveryAssessment::Unchanged
    } else {
        SysRecoveryAssessment::Blocked
    })
}

pub(super) async fn apply_split_dns_transition(
    host: &impl SplitDnsHost,
    previous: Option<&SysSplitDnsStateV1>,
    desired: Option<&SysSplitDnsStateV1>,
) -> Result<()> {
    if let Some(desired) = desired {
        host.apply_split_dns(&split_dns_request(desired)).await?;
    }
    if let Some(previous) = previous
        && desired.is_none_or(|desired| {
            desired.resource != previous.resource || desired.os_id != previous.os_id
        })
    {
        host.remove_split_dns(&split_dns_request(previous)).await?;
    }
    Ok(())
}

pub(super) async fn recover_split_dns_transition(
    host: &impl SplitDnsHost,
    previous: Option<&SysSplitDnsStateV1>,
    desired: Option<&SysSplitDnsStateV1>,
) -> Result<()> {
    if let Some(desired) = desired
        && split_dns_matches(host, desired).await?
        && previous.is_none_or(|previous| {
            previous.resource != desired.resource || previous.os_id != desired.os_id
        })
    {
        host.remove_split_dns(&split_dns_request(desired)).await?;
    }
    if let Some(previous) = previous {
        host.apply_split_dns(&split_dns_request(previous)).await?;
    }
    Ok(())
}

pub(super) fn split_dns_request(state: &SysSplitDnsStateV1) -> SplitDnsRequest {
    let marker = format!("Managed by shine: split-dns:{}", state.item_id);
    let content = if state.os_id == "windows" {
        format!(
            "{marker}\n{}\n{}",
            state.resource.display(),
            state.servers.join(",")
        )
        .into_bytes()
    } else if state.os_id == "macos" {
        format!(
            "# {marker}\n{}\n",
            state
                .servers
                .iter()
                .map(|server| format!("nameserver {server}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
        .into_bytes()
    } else {
        format!(
            "# {marker}\n[Resolve]\nDNS={}\nDomains=~{}\n",
            state.servers.join(" "),
            state.domain
        )
        .into_bytes()
    };
    SplitDnsRequest {
        os_id: state.os_id.clone(),
        item_id: state.item_id.clone(),
        domain: state.domain.clone(),
        servers: state.servers.clone(),
        resource: state.resource.clone(),
        content,
    }
}

pub(super) async fn split_dns_matches(
    host: &impl SplitDnsObservationHost,
    state: &SysSplitDnsStateV1,
) -> Result<bool> {
    let request = split_dns_request(state);
    let current = host.inspect_split_dns(&request).await?;
    Ok(current.exists
        && hash_content(&current.content) == state.content_hash
        && current.content == request.content)
}

pub(super) async fn split_dns_exists(
    host: &impl SplitDnsObservationHost,
    state: &SysSplitDnsStateV1,
) -> Result<bool> {
    Ok(host
        .inspect_split_dns(&split_dns_request(state))
        .await?
        .exists)
}
