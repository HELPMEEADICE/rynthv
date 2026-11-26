use super::Editor;
use crate::model::Language;
use crate::tr;

impl Editor {
    pub fn set_note_phonemes(&mut self, group_id: &str, notes: &[u64], phonemes: &str) {
        self.begin(tr!("编辑音素"));
        if let Some(group) = self.project.group_mut(group_id) {
            for note in &mut group.notes {
                if notes.contains(&note.id) {
                    note.replace_phonemes(phonemes);
                }
            }
        }
        self.finish();
    }

    pub fn set_note_language(&mut self, language: Option<Language>) {
        self.edit_selected_notes(tr!("切换音符语言"), |note| {
            note.set_language_override(language);
        });
    }

    pub fn set_even_syllables(&mut self, even: bool) {
        self.edit_selected_notes(tr!("调整音节分配"), |note| {
            note.attributes
                .insert("evenSyllableDuration".into(), even.into());
        });
    }

    fn edit_selected_notes(&mut self, label: &str, mut edit: impl FnMut(&mut crate::model::Note)) {
        if self.selected.is_empty() {
            return;
        }
        self.begin(label);
        let selected = self.selected.clone();
        if let Some(group) = self.group_mut() {
            for note in &mut group.notes {
                if selected.contains(&note.id) {
                    edit(note);
                }
            }
        }
        self.finish();
    }
}
