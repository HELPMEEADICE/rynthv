//! Build the execution arena while the original model resources are borrowed.

use std::{collections::HashMap, sync::Arc};

use crate::synthesis::{
    inference::{
        UnaryOperator,
        affine::AFFINE_TYPE_ID,
        convolution::{CONVOLUTION_TYPE_ID, TRANSPOSED_CONVOLUTION_TYPE_ID},
    },
    voice::{
        DURATION_ROOT_TYPE_ID, DurationConfiguration, PHONE_SET_TYPE_ID, PhoneSet,
        archive::Archive, array, embedded, tensor, tree::ModuleTree,
    },
};

use super::{
    DurationInputLayout, PreparedAffine, PreparedConvolution, PreparedModule, PreparedNode,
    PreparedTransposedConvolution, PreparedTree, Result, VoiceError, factory,
};

impl PreparedTree {
    pub(in crate::synthesis::voice) fn prepare(
        archive: &Archive,
        source: &ModuleTree,
    ) -> Result<Self> {
        let mut tensors = HashMap::<usize, usize>::new();
        let mut nodes: Vec<PreparedNode> = Vec::with_capacity(source.nodes.len());
        for node in &source.nodes {
            let index = archive
                .index(node.resource)
                .ok_or_else(|| VoiceError::new("Prepared node resource is absent"))?;
            let children: Vec<_> = node.children.iter().map(|child| child.index()).collect();
            let prepared = if let Some(&previous) = tensors.get(&index) {
                require_leaf(&children, "Tensor")?;
                let node = &nodes[previous];
                let PreparedModule::Tensor(storage) = &node.module else {
                    unreachable!("only tensors are cached");
                };
                PreparedNode {
                    resource_index: index,
                    type_id: node.type_id,
                    children,
                    payload_bytes: node.payload_bytes,
                    payload: None,
                    module: PreparedModule::Tensor(Arc::clone(storage)),
                }
            } else {
                let bytes = archive.read(index, None)?;
                let type_id = array(&bytes, 0)?;
                let prepared =
                    PreparedNode::prepare(index, type_id, &bytes[16..], children, &nodes)?;
                if matches!(prepared.module, PreparedModule::Tensor(_)) {
                    tensors.insert(index, nodes.len());
                }
                prepared
            };
            nodes.push(prepared);
        }
        Self::finish(nodes, source.root.index())
    }

    pub(crate) fn prepare_embedded(bytes: &[u8]) -> Result<Self> {
        Self::prepare_embedded_records(embedded::parse(bytes)?)
    }

    pub(in crate::synthesis::voice) fn prepare_flat_embedded(bytes: &[u8]) -> Result<Self> {
        Self::prepare_embedded_records(embedded::parse_bounded(bytes, 65_536, 128)?)
    }

    fn prepare_embedded_records(records: Vec<embedded::Record<'_>>) -> Result<Self> {
        let mut nodes = Vec::with_capacity(records.len());
        for (index, record) in records.into_iter().enumerate() {
            nodes.push(PreparedNode::prepare(
                index,
                record.type_id,
                record.payload,
                record.children,
                &nodes,
            )?);
        }
        let root = nodes
            .len()
            .checked_sub(1)
            .ok_or_else(|| VoiceError::new("Embedded DNNI archive is empty"))?;
        Self::finish(nodes, root)
    }

    fn finish(nodes: Vec<PreparedNode>, root: usize) -> Result<Self> {
        let node = nodes
            .get(root)
            .ok_or_else(|| VoiceError::new("Prepared root is outside its module arena"))?;
        let duration_input = if node.type_id == DURATION_ROOT_TYPE_ID {
            let config = DurationConfiguration::parse(node.payload.as_deref().unwrap_or_default())?;
            Some(DurationInputLayout::prepare(config, node, &nodes)?)
        } else {
            None
        };
        Ok(Self {
            nodes,
            root,
            duration_input,
        })
    }
}

impl PreparedNode {
    fn prepare(
        index: usize,
        type_id: [u8; 16],
        payload: &[u8],
        children: Vec<usize>,
        nodes: &[Self],
    ) -> Result<Self> {
        if children.iter().any(|&child| child >= nodes.len()) {
            return Err(VoiceError::new("Module arena is not in checked postorder"));
        }
        let module = if tensor::is_known(type_id) {
            require_leaf(&children, "Tensor")?;
            PreparedModule::Tensor(Arc::new(tensor::load(type_id, payload)?))
        } else if type_id == PHONE_SET_TYPE_ID {
            require_leaf(&children, "Phone set")?;
            PreparedModule::PhoneSet(PhoneSet::parse(payload)?)
        } else if let Ok(operator) = UnaryOperator::from_type_id(type_id) {
            require_empty_payload(payload)?;
            require_leaf(&children, "Unary operator")?;
            PreparedModule::Unary(operator)
        } else if type_id == AFFINE_TYPE_ID {
            require_empty_payload(payload)?;
            PreparedModule::Affine(PreparedAffine::prepare(nodes, &children)?)
        } else if type_id == CONVOLUTION_TYPE_ID {
            PreparedModule::Convolution(PreparedConvolution::prepare(payload, nodes, &children)?)
        } else if type_id == TRANSPOSED_CONVOLUTION_TYPE_ID {
            PreparedModule::TransposedConvolution(PreparedTransposedConvolution::prepare(
                payload, nodes, &children,
            )?)
        } else {
            factory::prepare_operator(type_id, payload, nodes, &children)?
                .unwrap_or(PreparedModule::Unsupported)
        };
        Ok(Self {
            resource_index: index,
            type_id,
            children,
            payload_bytes: payload.len(),
            payload: (!matches!(module, PreparedModule::Tensor(_))).then(|| payload.into()),
            module,
        })
    }
}

fn require_empty_payload(payload: &[u8]) -> Result<()> {
    if !payload.is_empty() {
        return Err(VoiceError::new(
            "Recovered parameterless module contains a payload",
        ));
    }
    Ok(())
}

fn require_leaf(children: &[usize], kind: &str) -> Result<()> {
    if !children.is_empty() {
        return Err(VoiceError::new(format!(
            "{kind} unexpectedly contains child modules"
        )));
    }
    Ok(())
}
