/// Запрос к читающей стороне. Как и команда, это просто данные — структура с параметрами;
/// `Output` задаёт тип ответа.
pub trait Query {
    type Output;
}

/// Отвечает на запросы типа `Q` по read-модели и ничего не меняет.
pub trait QueryHandler<Q: Query> {
    type Error;

    fn handle(&self, query: Q) -> impl Future<Output = Result<Q::Output, Self::Error>> + Send;
}
