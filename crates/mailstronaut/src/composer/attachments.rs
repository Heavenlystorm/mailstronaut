//! Native portal selection and bounded off-thread copies; never retain source paths.

use ashpd::desktop::{ResponseError, file_chooser::SelectedFiles};
use mailstronaut_core::attachments::read_files;
use mailstronaut_mail::compose::AttachmentData;

pub(super) async fn choose(
    remaining: usize,
    count: usize,
) -> Result<Option<Vec<AttachmentData>>, String> {
    let response = SelectedFiles::open_file()
        .title("Attach files")
        .modal(true)
        .multiple(true)
        .send()
        .await
        .map_err(|_| "File chooser unavailable. No files were attached.")?
        .response();
    let selected = match response {
        Ok(selected) => selected,
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => return Ok(None),
        Err(_) => return Err("File selection failed. No files were attached.".into()),
    };
    if selected.uris().len() > count {
        return Err("Too many attachments (maximum 16).".into());
    }
    let paths = selected
        .uris()
        .iter()
        .map(|uri| {
            uri.to_file_path()
                .map_err(|()| "Only locally accessible files can be attached.".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if paths.is_empty() {
        return Ok(None);
    }
    tokio::task::spawn_blocking(move || read_files(paths, remaining))
        .await
        .map_err(|_| "Attachment import interrupted. No files were attached.".to_owned())?
        .map(Some)
}

#[cfg(test)]
mod portal_tests;
