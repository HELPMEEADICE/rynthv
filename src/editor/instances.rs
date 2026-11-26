use super::Editor;
use crate::model::{EntityId, Group, GroupRef, Track, new_id};
use crate::tr;

impl Editor {
    pub fn copy_track(&mut self, track: EntityId) -> Option<EntityId> {
        let mut copy = self
            .project
            .tracks
            .iter()
            .find(|item| item.id == track)?
            .clone();
        self.begin(tr!("复制音轨"));
        copy.id = new_id();
        copy.name.push_str(" Copy");
        copy.disp_order = self.project.tracks.len() as i32;
        copy.main_group = copy_definition(&copy.main_group);
        copy.main_ref.group_id.clone_from(&copy.main_group.uuid);
        copy.main_ref.renew_identity();
        for reference in &mut copy.groups {
            reference.renew_identity();
        }
        let id = copy.id;
        self.project.tracks.push(copy);
        self.finish();
        Some(id)
    }

    pub fn copy_instance(
        &mut self,
        source: &GroupRef,
        track: EntityId,
        linked: bool,
        delta: i64,
    ) -> Result<EntityId, String> {
        let mut reference = shifted(source, delta)?;
        let group = if linked {
            None
        } else {
            let original = self
                .project
                .group(&source.group_id)
                .ok_or("Missing group definition")?;
            let group = copy_definition(original);
            reference.group_id.clone_from(&group.uuid);
            Some(group)
        };
        let index = self
            .project
            .tracks
            .iter()
            .position(|item| item.id == track && !item.is_instrumental())
            .ok_or("Target must be a vocal track")?;
        self.begin(tr!("复制音符组实例"));
        if let Some(group) = group {
            self.project.library.push(group);
        }
        reference.renew_identity();
        let id = reference.id;
        self.project.tracks[index].groups.push(reference);
        self.finish();
        Ok(id)
    }

    pub fn make_instance_independent(&mut self, id: EntityId) -> Result<(), String> {
        let source = self
            .project
            .tracks
            .iter()
            .flat_map(|track| &track.groups)
            .find(|reference| reference.id == id)
            .ok_or("Choose a non-main group instance")?;
        let group = copy_definition(
            self.project
                .group(&source.group_id)
                .ok_or("Missing group definition")?,
        );
        self.begin(tr!("独立化音符组实例"));
        self.project
            .tracks
            .iter_mut()
            .flat_map(|track| &mut track.groups)
            .find(|reference| reference.id == id)
            .expect("existing reference")
            .group_id
            .clone_from(&group.uuid);
        self.project.library.push(group);
        self.finish();
        Ok(())
    }

    pub fn split_instance(&mut self, id: EntityId, at: i64) -> Result<EntityId, String> {
        let (track, index) = self.instance_position(id)?;
        let mut right = self.project.tracks[track].groups[index].clone();
        if at <= right.blick_absolute_begin || at >= right.visible_end() {
            return Err("Split point must be inside the instance boundary".into());
        }
        right.blick_absolute_begin = at;
        right.renew_identity();
        let right_id = right.id;
        self.begin(tr!("拆分音符组实例"));
        self.project.tracks[track].groups[index].blick_absolute_end = at;
        self.project.tracks[track].groups.insert(index + 1, right);
        self.finish();
        Ok(right_id)
    }

    pub fn merge_instances(&mut self, first: EntityId, second: EntityId) -> Result<(), String> {
        let (track, left) = self.instance_position(first)?;
        let (other, right) = self.instance_position(second)?;
        let a = &self.project.tracks[track].groups[left];
        let b = &self.project.tracks[other].groups[right];
        let mut matching = a.clone();
        matching.id = b.id;
        matching.uuid.clone_from(&b.uuid);
        matching.blick_absolute_begin = b.blick_absolute_begin;
        matching.blick_absolute_end = b.blick_absolute_end;
        if track != other
            || first == second
            || a.blick_absolute_end != b.blick_absolute_begin
            || &matching != b
        {
            return Err(
                "Merge requires adjacent, identically configured instances of the same definition"
                    .into(),
            );
        }
        let end = b.blick_absolute_end;
        self.begin(tr!("合并音符组实例"));
        self.project.tracks[track].groups[left].blick_absolute_end = end;
        self.project.tracks[track].groups.remove(right);
        self.finish();
        Ok(())
    }

    pub fn move_instance(
        &mut self,
        id: EntityId,
        track: EntityId,
        delta: i64,
    ) -> Result<(), String> {
        let (source, index) = self.instance_position(id)?;
        let target = self
            .project
            .tracks
            .iter()
            .position(|item| item.id == track && !item.is_instrumental())
            .ok_or("Target must be a vocal track")?;
        let reference = shifted(&self.project.tracks[source].groups[index], delta)?;
        self.begin(tr!("移动音符组实例"));
        if source == target {
            self.project.tracks[source].groups[index] = reference;
        } else {
            self.project.tracks[source].groups.remove(index);
            self.project.tracks[target].groups.push(reference);
        }
        self.finish();
        Ok(())
    }

    fn instance_position(&self, id: EntityId) -> Result<(usize, usize), String> {
        self.project
            .tracks
            .iter()
            .enumerate()
            .find_map(|(track, item): (usize, &Track)| {
                item.groups
                    .iter()
                    .position(|reference| reference.id == id)
                    .map(|index| (track, index))
            })
            .ok_or_else(|| "Choose a non-main group instance".into())
    }
}

fn shifted(reference: &GroupRef, delta: i64) -> Result<GroupRef, String> {
    let mut result = reference.clone();
    let shift = |value: i64| value.checked_add(delta).ok_or("Group position overflow");
    result.blick_absolute_begin = shift(result.blick_absolute_begin)?;
    if result.blick_absolute_begin < 0 {
        return Err("Group must start at a nonnegative project position".into());
    }
    if result.blick_absolute_end >= 0 {
        result.blick_absolute_end = shift(result.blick_absolute_end)?;
    }
    result.blick_offset = shift(result.blick_offset)?;
    Ok(result)
}

fn copy_definition(source: &Group) -> Group {
    let mut group = source.clone();
    group.uuid = uuid::Uuid::new_v4().to_string();
    for note in &mut group.notes {
        note.renew_identity();
    }
    group
}
