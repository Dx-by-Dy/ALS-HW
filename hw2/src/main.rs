use rust_decimal::Decimal;

use hw2::application::order_service::OrderService;
use hw2::domain::pricing::PricingPolicy;
use hw2::domain::order::{ItemId, Order, OrderId, OrderItem};
use hw2::services::email_service::EmailService;
use hw2::types::{discount::Discount, money::Money, tax::Tax};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = OrderService::new(EmailService::new());

    let pricing = PricingPolicy::discount_then_tax(
        Discount::percent_off_above(Decimal::from(10), Money::from(Decimal::from(100))),
        Tax::percent(Decimal::from(20)),
    );

    let mut order = Order::new(OrderId::from(1));
    order.add_item(OrderItem::new(ItemId::from(1), 2, Money::from(Decimal::from(50)))?)?;
    order.add_item(OrderItem::new(ItemId::from(2), 1, Money::from(Decimal::from(30)))?)?;

    println!("order {} total: {}", order.id(), order.total_with(&pricing));

    if let Some(failure) = service.pay(&mut order)? {
        eprintln!("{failure}");
    }
    if let Some(failure) = service.post(&mut order)? {
        eprintln!("{failure}");
    }
    println!("order {} status: {}", order.id(), order.status());

    Ok(())
}
