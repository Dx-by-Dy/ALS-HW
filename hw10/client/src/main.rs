use client::model::{http::BrowserHttpClient, ticket_api::TicketApiService};
use client::view::TicketView;
use client::view_model::TicketViewModel;
use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(|| {
        let vm = TicketViewModel::new(TicketApiService::new(BrowserHttpClient));
        view! { <TicketView vm /> }
    });
}
