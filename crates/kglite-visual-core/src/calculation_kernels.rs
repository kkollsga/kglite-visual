//! Kernels consume only the bounded visible relation multiset, never source adjacency.
use std::collections::BTreeMap;
use std::time::Instant;

use crate::records::{MAX_LOADED_EDGES, MAX_LOADED_NODES};
use crate::CoreError;

pub(crate) struct CalculationInput {
    pub node_ids: Vec<u32>,
    pub edges: Vec<(usize, usize)>,
}
impl CalculationInput {
    fn validate(&self) -> Result<(), CoreError> {
        if self.node_ids.len() > MAX_LOADED_NODES || self.edges.len() > MAX_LOADED_EDGES {
            return Err(refusal(
                "calculation input exceeds 5000 nodes or 20000 relation records",
            ));
        }
        if self.node_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(refusal(
                "calculation node identities must be unique and sorted",
            ));
        }
        if self
            .edges
            .iter()
            .any(|&(source, target)| source >= self.node_ids.len() || target >= self.node_ids.len())
        {
            return Err(refusal(
                "calculation relation endpoint is outside its captured input",
            ));
        }
        Ok(())
    }
}

pub(crate) fn degree(
    input: &CalculationInput,
    deadline: Option<Instant>,
) -> Result<Vec<[u32; 3]>, CoreError> {
    input.validate()?;
    check_deadline(deadline)?;
    let mut values = vec![[0u32; 3]; input.node_ids.len()];
    for &(source, target) in &input.edges {
        check_deadline(deadline)?;
        values[source][1] = values[source][1]
            .checked_add(1)
            .ok_or_else(|| refusal("out-degree overflow"))?;
        values[target][0] = values[target][0]
            .checked_add(1)
            .ok_or_else(|| refusal("in-degree overflow"))?;
    }
    for value in &mut values {
        value[2] = value[0]
            .checked_add(value[1])
            .ok_or_else(|| refusal("total-degree overflow"))?;
    }
    Ok(values)
}

pub(crate) fn weak_components(
    input: &CalculationInput,
    deadline: Option<Instant>,
) -> Result<Vec<[u32; 2]>, CoreError> {
    input.validate()?;
    check_deadline(deadline)?;
    let mut parent: Vec<usize> = (0..input.node_ids.len()).collect();
    let mut size = vec![1usize; parent.len()];
    for &(source, target) in &input.edges {
        check_deadline(deadline)?;
        let mut a = root(&mut parent, source);
        let mut b = root(&mut parent, target);
        if a == b {
            continue;
        }
        if size[a] < size[b] {
            std::mem::swap(&mut a, &mut b);
        }
        parent[b] = a;
        size[a] += size[b];
    }
    let mut labels = BTreeMap::new();
    let mut values = Vec::with_capacity(parent.len());
    // Input order is ascending source identity, so the first member of a
    // component assigns its stable label independently of union tree shape.
    for node in 0..parent.len() {
        check_deadline(deadline)?;
        let root = root(&mut parent, node);
        let next = labels.len() as u32 + 1;
        let label = *labels.entry(root).or_insert(next);
        values.push([label, size[root] as u32]);
    }
    Ok(values)
}

fn root(parent: &mut [usize], mut at: usize) -> usize {
    while parent[at] != at {
        parent[at] = parent[parent[at]];
        at = parent[at];
    }
    at
}

pub(crate) fn check_deadline(deadline: Option<Instant>) -> Result<(), CoreError> {
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(refusal(
            "visible-subset calculation exceeded its work deadline",
        ));
    }
    Ok(())
}
fn refusal(message: &str) -> CoreError {
    CoreError::Request(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directed_parallel_edges_and_self_loop_count_separately() {
        let input = CalculationInput {
            node_ids: vec![3, 8, 11],
            edges: vec![(0, 1), (0, 1), (1, 1)],
        };
        assert_eq!(
            degree(&input, None).unwrap(),
            vec![[0, 2, 2], [3, 1, 4], [0, 0, 0]]
        );
        assert_eq!(
            weak_components(&input, None).unwrap(),
            vec![[1, 2], [1, 2], [2, 1]]
        );
    }

    #[test]
    fn weak_components_ignore_direction_and_edge_order_but_preserve_isolates() {
        let input = CalculationInput {
            node_ids: vec![2, 5, 8, 13, 21],
            edges: vec![(3, 1), (4, 0), (1, 3)],
        };
        assert_eq!(
            weak_components(&input, None).unwrap(),
            vec![[1, 2], [2, 2], [3, 1], [2, 2], [1, 2]]
        );
        let reversed = CalculationInput {
            node_ids: input.node_ids.clone(),
            edges: input.edges.iter().rev().map(|&(a, b)| (b, a)).collect(),
        };
        assert_eq!(
            weak_components(&input, None).unwrap(),
            weak_components(&reversed, None).unwrap()
        );
    }

    #[test]
    fn empty_input_and_bounds_have_explicit_results() {
        let empty = CalculationInput {
            node_ids: Vec::new(),
            edges: Vec::new(),
        };
        assert!(degree(&empty, None).unwrap().is_empty());
        assert!(weak_components(&empty, None).unwrap().is_empty());
        assert!(degree(&empty, Some(Instant::now())).is_err());
        let too_many = CalculationInput {
            node_ids: (0..=MAX_LOADED_NODES as u32).collect(),
            edges: Vec::new(),
        };
        assert!(degree(&too_many, None).is_err());
        let outside = CalculationInput {
            node_ids: vec![1],
            edges: vec![(0, 1)],
        };
        assert!(weak_components(&outside, None).is_err());
    }
}
