/// Что сага просит сделать в ответ на событие или дедлайн.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effects<C, D, T> {
    /// Команды, в порядке отправки.
    pub commands: Vec<C>,
    /// Какие дедлайны назначить. Дедлайн с тем же именем заменяется.
    pub deadlines: Vec<(D, T)>,
    /// Процесс завершён: экземпляр и все его дедлайны удаляются.
    pub finished: bool,
}

impl<C, D, T> Effects<C, D, T> {
    /// Ничего делать не нужно.
    pub fn none() -> Self {
        Self {
            commands: Vec::new(),
            deadlines: Vec::new(),
            finished: false,
        }
    }

    pub fn command(mut self, command: C) -> Self {
        self.commands.push(command);
        self
    }

    pub fn schedule(mut self, deadline: D, at: T) -> Self {
        self.deadlines.push((deadline, at));
        self
    }

    pub fn finish(mut self) -> Self {
        self.finished = true;
        self
    }
}
