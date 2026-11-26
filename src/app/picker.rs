use super::App;
use eframe::egui::Context;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub(crate) enum Selection {
    File,
    Files,
    Directory,
    #[cfg(target_os = "android")]
    Publish,
}

pub(crate) struct Picker {
    selection: Selection,
    title: &'static str,
    filter: Option<(&'static str, &'static [&'static str])>,
    directory: Option<PathBuf>,
    #[cfg(target_os = "android")]
    publish: Option<(PathBuf, &'static str)>,
}

impl Picker {
    pub(crate) fn new(selection: Selection, title: &'static str) -> Self {
        Self {
            selection,
            title,
            filter: None,
            directory: None,
            #[cfg(target_os = "android")]
            publish: None,
        }
    }

    #[cfg(target_os = "android")]
    pub(crate) fn publish(path: PathBuf, mime: &'static str) -> Self {
        let mut picker = Self::new(Selection::Publish, crate::tr!("保存副本"));
        picker.publish = Some((path, mime));
        picker
    }

    pub(crate) fn filter(
        mut self,
        label: &'static str,
        extensions: &'static [&'static str],
    ) -> Self {
        self.filter = Some((label, extensions));
        self
    }

    pub(crate) fn starting_at(mut self, directory: Option<&Path>) -> Self {
        self.directory = directory.map(Path::to_owned);
        self
    }

    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    fn pick(self) -> Vec<PathBuf> {
        let mut dialog = rfd::FileDialog::new().set_title(self.title);
        if let Some((label, extensions)) = self.filter {
            dialog = dialog.add_filter(label, extensions);
        }
        if let Some(directory) = self.directory {
            dialog = dialog.set_directory(directory);
        }
        match self.selection {
            Selection::File => dialog.pick_file().into_iter().collect(),
            Selection::Files => dialog.pick_files().unwrap_or_default(),
            Selection::Directory => dialog.pick_folder().into_iter().collect(),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn start(self) -> js_sys::Promise {
        let kind = match self.selection {
            Selection::File => 0,
            Selection::Files => 1,
            Selection::Directory => 2,
        };
        let accept = self
            .filter
            .map(|(_, extensions)| {
                extensions
                    .iter()
                    .map(|ext| format!(".{ext}"))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        crate::platform::web::documents::pick_files(kind, &accept, self.title)
    }

    #[cfg(target_os = "android")]
    fn start(self) -> Result<(), String> {
        use crate::platform::android::with_activity;
        let kind = match self.selection {
            Selection::File => 0,
            Selection::Files => 1,
            Selection::Directory => 2,
            Selection::Publish => 3,
        };
        let extensions = self
            .filter
            .map_or_else(String::new, |(_, list)| list.join(","));
        with_activity(|env, activity| {
            let title = env.new_string(self.title)?;
            let extensions = env.new_string(&extensions)?;
            let (source, mime) = self.publish.as_ref().map_or_else(
                || (std::borrow::Cow::Borrowed(""), ""),
                |(path, mime)| (path.to_string_lossy(), *mime),
            );
            let source = env.new_string(&source)?;
            let mime = env.new_string(mime)?;
            env.call_method(
                activity,
                jni::jni_str!("rynthvPick"),
                jni::jni_sig!(
                    "(ILjava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V"
                ),
                &[
                    kind.into(),
                    (&title).into(),
                    (&extensions).into(),
                    (&source).into(),
                    (&mime).into(),
                ],
            )?;
            Ok(())
        })
    }
}

#[cfg(any(target_os = "android", target_arch = "wasm32"))]
pub(super) type Completion = Box<dyn FnOnce(&mut App, &Context, Vec<PathBuf>)>;

impl App {
    pub(crate) fn pick_one(
        &mut self,
        ctx: &Context,
        picker: Picker,
        complete: impl FnOnce(&mut Self, &Context, PathBuf) + 'static,
    ) {
        self.pick(ctx, picker, |app, ctx, paths| {
            if let Some(path) = paths.into_iter().next() {
                complete(app, ctx, path);
            }
        });
    }

    pub(crate) fn pick(
        &mut self,
        _ctx: &Context,
        picker: Picker,
        complete: impl FnOnce(&mut Self, &Context, Vec<PathBuf>) + 'static,
    ) {
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        {
            let paths = picker.pick();
            if !paths.is_empty() {
                complete(self, _ctx, paths);
            }
        }
        #[cfg(target_os = "android")]
        {
            if self.picker.is_some() {
                return;
            }
            match picker.start() {
                Ok(()) => self.picker = Some(Box::new(complete)),
                Err(error) => self.error = Some(error),
            }
        }
        #[cfg(target_arch = "wasm32")]
        if self.browser.picker.is_none() {
            let promise = picker.start();
            self.browser.picker = Some((
                Box::new(complete),
                super::web::task(_ctx, crate::platform::web::documents::json(promise)),
            ));
        }
    }

    #[cfg(target_os = "android")]
    pub(super) fn poll_picker(&mut self, ctx: &Context) {
        use crate::platform::android::string_method;
        if self.picker.is_none() {
            return;
        }
        let result = string_method(jni::jni_str!("rynthvTakePickerResult")).and_then(|result| {
            result
                .map(|result| {
                    serde_json::from_str::<Result<Vec<PathBuf>, String>>(&result)
                        .map_err(|error| error.to_string())?
                })
                .transpose()
        });
        match result {
            Ok(Some(paths)) => {
                if let Some(complete) = self.picker.take()
                    && !paths.is_empty()
                {
                    complete(self, ctx, paths);
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.picker = None;
                self.error = Some(error);
            }
        }
    }
}
