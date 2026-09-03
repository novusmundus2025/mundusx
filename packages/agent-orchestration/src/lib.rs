use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{mpsc, Arc};
use std::thread;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: String,
    pub prompt: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: String,
    pub output: String,
}

pub trait TaskExecutor: Send + Sync + 'static {
    fn execute(&self, task: &AgentTask, inputs: &[TaskResult]) -> Result<String, String>;
}

#[derive(Debug, PartialEq)]
pub enum OrchestrationError {
    InvalidGraph(String),
    TaskFailed { task_id: String, message: String },
}

pub struct Orchestrator<E> {
    executor: Arc<E>,
    max_parallel: usize,
}

impl<E: TaskExecutor> Orchestrator<E> {
    pub fn new(executor: E, max_parallel: usize) -> Self {
        Self {
            executor: Arc::new(executor),
            max_parallel: max_parallel.clamp(1, 32),
        }
    }

    pub fn run(&self, tasks: Vec<AgentTask>) -> Result<Vec<TaskResult>, OrchestrationError> {
        validate_graph(&tasks)?;
        let ordered_ids = tasks.iter().map(|task| task.id.clone()).collect::<Vec<_>>();
        let mut pending = tasks
            .into_iter()
            .map(|task| (task.id.clone(), task))
            .collect::<BTreeMap<_, _>>();
        let mut completed = BTreeMap::<String, TaskResult>::new();

        while !pending.is_empty() {
            let ready = pending
                .values()
                .filter(|task| {
                    task.dependencies
                        .iter()
                        .all(|id| completed.contains_key(id))
                })
                .take(self.max_parallel)
                .cloned()
                .collect::<Vec<_>>();
            if ready.is_empty() {
                return Err(OrchestrationError::InvalidGraph(
                    "task graph contains a dependency cycle".to_string(),
                ));
            }
            let (sender, receiver) = mpsc::channel();
            let count = ready.len();
            for task in ready {
                pending.remove(&task.id);
                let inputs = task
                    .dependencies
                    .iter()
                    .filter_map(|id| completed.get(id).cloned())
                    .collect::<Vec<_>>();
                let executor = Arc::clone(&self.executor);
                let sender = sender.clone();
                thread::spawn(move || {
                    let result = executor.execute(&task, &inputs);
                    let _ = sender.send((task.id, result));
                });
            }
            drop(sender);
            for _ in 0..count {
                let (task_id, result) = receiver.recv().map_err(|error| {
                    OrchestrationError::InvalidGraph(format!("worker channel closed: {error}"))
                })?;
                let output = result.map_err(|message| OrchestrationError::TaskFailed {
                    task_id: task_id.clone(),
                    message,
                })?;
                completed.insert(task_id.clone(), TaskResult { task_id, output });
            }
        }

        Ok(ordered_ids
            .into_iter()
            .filter_map(|id| completed.remove(&id))
            .collect())
    }
}

fn validate_graph(tasks: &[AgentTask]) -> Result<(), OrchestrationError> {
    if tasks.is_empty() {
        return Err(OrchestrationError::InvalidGraph(
            "at least one task is required".to_string(),
        ));
    }
    let ids = tasks
        .iter()
        .map(|task| task.id.as_str())
        .collect::<BTreeSet<_>>();
    if ids.len() != tasks.len() || ids.contains("") {
        return Err(OrchestrationError::InvalidGraph(
            "task ids must be unique and non-empty".to_string(),
        ));
    }
    for task in tasks {
        if task
            .dependencies
            .iter()
            .any(|id| !ids.contains(id.as_str()))
        {
            return Err(OrchestrationError::InvalidGraph(format!(
                "task {} has an unknown dependency",
                task.id
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;
    impl TaskExecutor for Echo {
        fn execute(&self, task: &AgentTask, inputs: &[TaskResult]) -> Result<String, String> {
            Ok(format!("{}:{}", task.prompt, inputs.len()))
        }
    }

    #[test]
    fn executes_dependency_graph_and_preserves_declared_order() {
        let tasks = vec![
            AgentTask {
                id: "inspect".into(),
                prompt: "inspect".into(),
                dependencies: vec![],
            },
            AgentTask {
                id: "test".into(),
                prompt: "test".into(),
                dependencies: vec![],
            },
            AgentTask {
                id: "review".into(),
                prompt: "review".into(),
                dependencies: vec!["inspect".into(), "test".into()],
            },
        ];
        let results = Orchestrator::new(Echo, 2).run(tasks).expect("orchestrated");
        assert_eq!(
            results
                .iter()
                .map(|result| result.task_id.as_str())
                .collect::<Vec<_>>(),
            vec!["inspect", "test", "review"]
        );
        assert_eq!(results[2].output, "review:2");
    }

    #[test]
    fn rejects_cycles() {
        let tasks = vec![
            AgentTask {
                id: "a".into(),
                prompt: String::new(),
                dependencies: vec!["b".into()],
            },
            AgentTask {
                id: "b".into(),
                prompt: String::new(),
                dependencies: vec!["a".into()],
            },
        ];
        assert!(matches!(
            Orchestrator::new(Echo, 2).run(tasks),
            Err(OrchestrationError::InvalidGraph(_))
        ));
    }
}
