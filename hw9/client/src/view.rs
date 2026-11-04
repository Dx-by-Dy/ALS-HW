//! View: разметка формы. Своей логики здесь нет — только привязки к свойствам и командам
//! `TicketViewModel` и форматирование значений для показа.

use std::time::Duration;

use leptos::{prelude::*, task::spawn_local};

use api::{TicketDto, TicketStatus};

use crate::model::http::BrowserHttpClient;
use crate::model::ticket_api::TicketQuery;
use crate::view_model::TicketViewModel;

type Vm = TicketViewModel<BrowserHttpClient>;

/// Сколько дней показывает запрос закрытых тикетов, пока пользователь не ввёл своё.
const DEFAULT_CLOSED_DAYS: u64 = 7;

#[component]
pub fn TicketView(vm: Vm) -> impl IntoView {
    spawn_local(async move { vm.sync_forever(sleep).await });
    view! {
        <main class="app">
            <header class="topbar">
                <h1>"Support tickets"</h1>
                <span class="live" class:offline=move || !vm.is_connected()>
                    {move || match (vm.is_connected(), vm.version()) {
                        (true, Some(version)) => format!("live · v{version}"),
                        (false, Some(_)) => "offline · reconnecting…".to_owned(),
                        (_, None) => "connecting…".to_owned(),
                    }}
                </span>
                <span class="sync" class:visible=move || vm.is_busy()>
                    "Waiting for the server…"
                </span>
            </header>
            <TenantBar vm />
            <div class="layout">
                <TicketList vm />
                <TicketDetails vm />
            </div>
            <QueryPanel vm />
        </main>
    }
}

#[component]
fn TenantBar(vm: Vm) -> impl IntoView {
    view! {
        <section class="tenants">
            {move || {
                vm.tenants()
                    .into_iter()
                    .map(|tenant| {
                        let discounted = tenant.discount_percent > 0;
                        view! {
                            <div class="tenant" class:discounted=discounted>
                                <span class="tenant-name">{tenant.tenant_name}</span>
                                <span>{format!("{} open tickets", tenant.open_tickets)}</span>
                                <span class="discount">
                                    {format!("discount {}%", tenant.discount_percent)}
                                </span>
                            </div>
                        }
                    })
                    .collect_view()
            }}
        </section>
    }
}

#[component]
fn TicketList(vm: Vm) -> impl IntoView {
    view! {
        <section class="panel list">
            <table>
                <thead>
                    <tr>
                        <th>"#"</th>
                        <th>"Tenant"</th>
                        <th>"Category"</th>
                        <th>"Description"</th>
                        <th>"Status"</th>
                        <th>"Agent"</th>
                    </tr>
                </thead>
                <tbody>
                    {move || {
                        vm.tickets().into_iter().map(|ticket| ticket_row(vm, ticket)).collect_view()
                    }}
                </tbody>
            </table>
        </section>
    }
}

fn ticket_row(vm: Vm, ticket: TicketDto) -> impl IntoView {
    let id = ticket.ticket_id;
    let tenant = vm.tenant(ticket.tenant_id).map(|t| t.tenant_name);
    view! {
        <tr class:selected=move || vm.is_selected(id) on:click=move |_| vm.select_ticket(id)>
            <td>{id}</td>
            <td>{tenant}</td>
            <td>{ticket.category}</td>
            <td>{ticket.description}</td>
            <td>{status_badge(ticket.status)}</td>
            <td>{agent_name(vm, ticket.assigned_agent)}</td>
        </tr>
    }
}

#[component]
fn TicketDetails(vm: Vm) -> impl IntoView {
    view! {
        <section class="panel details">
            {move || match vm.selected() {
                None => view! { <p class="empty">"Select a ticket"</p> }.into_any(),
                Some(ticket) => ticket_card(vm, ticket).into_any(),
            }}
            <div class="actions">
                <button
                    on:click=move |_| spawn_local(async move { vm.open_ticket().await })
                    disabled=move || !vm.can_open()
                >
                    "Open"
                </button>
                <button
                    class="danger"
                    on:click=move |_| spawn_local(async move { vm.close_ticket().await })
                    disabled=move || !vm.can_close()
                >
                    "Close"
                </button>
            </div>
            <div class="assign">
                <label for="assignee">"Assign agent"</label>
                <div class="assign-row">
                    <select
                        id="assignee"
                        on:change=move |ev| {
                            vm.select_user(event_target_value(&ev).parse().ok())
                        }
                    >
                        <option value="">"Choose a user"</option>
                        {move || {
                            vm.users()
                                .into_iter()
                                .map(|user| {
                                    let id = user.user_id;
                                    view! {
                                        <option
                                            value=id.to_string()
                                            selected=move || vm.selected_user_id() == Some(id)
                                        >
                                            {format!("{} ({})", user.user_name, user.role)}
                                        </option>
                                    }
                                })
                                .collect_view()
                        }}
                    </select>
                    <button
                        on:click=move |_| spawn_local(async move { vm.assign_agent().await })
                        disabled=move || !vm.can_assign()
                    >
                        "Assign"
                    </button>
                </div>
                <p class="hint">{move || vm.assign_hint()}</p>
            </div>
            <p class="error" role="alert">{move || vm.error()}</p>
        </section>
    }
}

fn ticket_card(vm: Vm, ticket: TicketDto) -> impl IntoView {
    let tenant = vm.tenant(ticket.tenant_id).map(|t| t.tenant_name);
    view! {
        <div class="card">
            <div class="card-title">
                <h2>{format!("ticket-{} · {}", ticket.ticket_id, ticket.category)}</h2>
                {status_badge(ticket.status)}
            </div>
            <p class="description">{ticket.description}</p>
            <dl>
                <dt>"Tenant"</dt>
                <dd>{tenant}</dd>
                <dt>"Agent"</dt>
                <dd>{agent_name(vm, ticket.assigned_agent)}</dd>
                <dt>"Created"</dt>
                <dd>{format_utc(ticket.created_at)}</dd>
                <dt>"Closed"</dt>
                <dd>{ticket.closed_at.map_or_else(|| "—".to_owned(), format_utc)}</dd>
            </dl>
        </div>
    }
}

/// Запросы к read-модели сервера. Результат обновляется сам, когда меняются данные.
#[component]
fn QueryPanel(vm: Vm) -> impl IntoView {
    let run = move |query| spawn_local(async move { vm.run_query(query).await });
    let days = RwSignal::new(DEFAULT_CLOSED_DAYS);
    let is_active = move |pick: fn(TicketQuery) -> bool| vm.query().is_some_and(pick);
    view! {
        <section class="panel queries">
            <h2>"Queries"</h2>
            <div class="query-row" class:active=move || is_active(|q| matches!(q, TicketQuery::ActiveOfAgent(_)))>
                <span>"Active tickets of"</span>
                <select on:change=move |ev| {
                    if let Ok(agent_id) = event_target_value(&ev).parse() {
                        run(TicketQuery::ActiveOfAgent(agent_id));
                    }
                }>
                    <option value="">"Choose an agent"</option>
                    {move || {
                        vm.agents()
                            .into_iter()
                            .map(|agent| {
                                let query = TicketQuery::ActiveOfAgent(agent.user_id);
                                view! {
                                    <option
                                        value=agent.user_id.to_string()
                                        selected=move || vm.query() == Some(query)
                                    >
                                        {agent.user_name}
                                    </option>
                                }
                            })
                            .collect_view()
                    }}
                </select>
            </div>
            <div class="query-row" class:active=move || is_active(|q| matches!(q, TicketQuery::UnassignedOfTenant(_)))>
                <span>"Unassigned tickets of"</span>
                <select on:change=move |ev| {
                    if let Ok(tenant_id) = event_target_value(&ev).parse() {
                        run(TicketQuery::UnassignedOfTenant(tenant_id));
                    }
                }>
                    <option value="">"Choose a tenant"</option>
                    {move || {
                        vm.tenants()
                            .into_iter()
                            .map(|tenant| {
                                let query = TicketQuery::UnassignedOfTenant(tenant.tenant_id);
                                view! {
                                    <option
                                        value=tenant.tenant_id.to_string()
                                        selected=move || vm.query() == Some(query)
                                    >
                                        {tenant.tenant_name}
                                    </option>
                                }
                            })
                            .collect_view()
                    }}
                </select>
            </div>
            <div class="query-row" class:active=move || is_active(|q| matches!(q, TicketQuery::ClosedWithin(_)))>
                <span>"Closed in the last"</span>
                <input
                    type="number"
                    min="1"
                    prop:value=move || days.get().to_string()
                    on:change=move |ev| {
                        if let Ok(value) = event_target_value(&ev).parse::<u64>() {
                            days.set(value.max(1));
                            run(TicketQuery::ClosedWithin(days.get_untracked()));
                        }
                    }
                />
                <span>"days"</span>
                <button on:click=move |_| run(TicketQuery::ClosedWithin(days.get_untracked()))>
                    "Show"
                </button>
            </div>
            {move || match (vm.query(), vm.query_error()) {
                (None, _) => {
                    view! { <p class="empty">"Choose a query to ask the read model"</p> }.into_any()
                }
                (Some(_), Some(error)) => view! { <p class="error">{error}</p> }.into_any(),
                (Some(_), None) => query_results(vm, vm.query_results()).into_any(),
            }}
        </section>
    }
}

fn query_results(vm: Vm, tickets: Vec<TicketDto>) -> impl IntoView {
    if tickets.is_empty() {
        return view! { <p class="empty">"No tickets"</p> }.into_any();
    }
    view! {
        <table>
            <thead>
                <tr>
                    <th>"#"</th>
                    <th>"Tenant"</th>
                    <th>"Description"</th>
                    <th>"Status"</th>
                    <th>"Agent"</th>
                    <th>"Closed"</th>
                </tr>
            </thead>
            <tbody>
                {tickets
                    .into_iter()
                    .map(|ticket| {
                        let id = ticket.ticket_id;
                        let tenant = vm.tenant(ticket.tenant_id).map(|t| t.tenant_name);
                        view! {
                            <tr on:click=move |_| vm.select_ticket(id)>
                                <td>{id}</td>
                                <td>{tenant}</td>
                                <td>{ticket.description}</td>
                                <td>{status_badge(ticket.status)}</td>
                                <td>{agent_name(vm, ticket.assigned_agent)}</td>
                                <td>{ticket.closed_at.map_or_else(|| "—".to_owned(), format_utc)}</td>
                            </tr>
                        }
                    })
                    .collect_view()}
            </tbody>
        </table>
    }
    .into_any()
}

fn status_badge(status: TicketStatus) -> impl IntoView {
    let class = match status {
        TicketStatus::Open => "badge open",
        TicketStatus::InProgress => "badge in-progress",
        TicketStatus::Closed => "badge closed",
    };
    view! { <span class=class>{status.to_string()}</span> }
}

fn agent_name(vm: Vm, agent: Option<usize>) -> String {
    agent
        .and_then(|id| vm.user_name(id))
        .unwrap_or_else(|| "—".to_owned())
}

/// Пауза поверх `setTimeout` браузера — таймер для повторного подключения.
async fn sleep(duration: Duration) {
    let (done, elapsed) = futures::channel::oneshot::channel();
    set_timeout(
        move || {
            let _ = done.send(());
        },
        duration,
    );
    let _ = elapsed.await;
}

/// Секунды Unix-времени как дата и время UTC: `2026-10-27 18:42 UTC`.
fn format_utc(secs: u64) -> String {
    // Перевод дней от эпохи в григорианскую дату (алгоритм Говарда Хиннанта).
    let days = (secs / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let time = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        time / 3_600,
        time % 3_600 / 60
    )
}

#[cfg(test)]
mod tests {
    use super::format_utc;

    #[test]
    fn unix_seconds_are_shown_as_utc_date_and_time() {
        assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(format_utc(1_793_134_069), "2026-10-27 20:47 UTC");
    }
}
