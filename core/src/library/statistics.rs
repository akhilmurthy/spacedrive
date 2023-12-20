use crate::{api::utils::get_size, library::Library, volume::get_volumes, Node};

use chrono::Utc;
use sd_prisma::prisma::statistics;
use std::sync::Arc;
use tracing::info;

use super::LibraryManagerError;

pub async fn update_library_statistics(
	node: Arc<Node>,
	library: Arc<Library>,
) -> Result<statistics::Data, LibraryManagerError> {
	let volumes = get_volumes().await;

	let mut total_capacity: u64 = 0;
	let mut available_capacity: u64 = 0;
	for volume in volumes {
		total_capacity += volume.total_capacity;
		available_capacity += volume.available_capacity;
	}

	let total_bytes_used = total_capacity - available_capacity;

	let library_db_size = get_size(
		node.config
			.data_directory()
			.join("libraries")
			.join(&format!("{}.db", library.id)),
	)
	.await
	.unwrap_or(0);

	let thumbnail_folder_size = get_size(node.config.data_directory().join("thumbnails"))
		.await
		.unwrap_or(0);

	use statistics::*;
	let params = vec![
		id::set(1), // Each library is a database so only one of these ever exists
		date_captured::set(Utc::now().into()),
		total_object_count::set(0),
		library_db_size::set(library_db_size.to_string()),
		total_bytes_used::set(total_bytes_used.to_string()),
		total_bytes_capacity::set(total_capacity.to_string()),
		total_unique_bytes::set(0.to_string()),
		total_bytes_free::set(available_capacity.to_string()),
		preview_media_bytes::set(thumbnail_folder_size.to_string()),
	];

	let stats = library
		.db
		.statistics()
		.upsert(
			// Each library is a database so only one of these ever exists
			statistics::id::equals(1),
			statistics::create(params.clone()),
			params,
		)
		.exec()
		.await?;

	info!("Updated library statistics: {:?}", stats);

	Ok(stats)
}
