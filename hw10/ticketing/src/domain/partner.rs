use std::fmt;

/// Партнёр — внешняя организация, которой передают тикет, если своих ресурсов не хватает.
///
/// Агрегата у партнёра нет: система тикетов знает только его идентификатор, по которому
/// партнёра узнаёт Open-Host Service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartnerId(pub usize);

impl fmt::Display for PartnerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "partner-{}", self.0)
    }
}
