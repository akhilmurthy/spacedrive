use std::sync::Arc;

use itertools::{Either, Itertools};
use sd_p2p::{
	proto::{decode, encode},
	spacetunnel::Tunnel,
};
use sd_sync::CRDTOperation;
use sync::GetOpsArgs;

use tokio::{
	io::{AsyncRead, AsyncWrite, AsyncWriteExt},
	sync::broadcast,
};
use tracing::*;
use uuid::Uuid;

use crate::{
	library::{Libraries, Library, LibraryManagerEvent},
	sync,
};

use super::{Header, IdentityOrRemoteIdentity, P2PManager};

mod proto;
pub use proto::*;

pub(crate) async fn networked_libraries_v2(
	manager: Arc<P2PManager>,
	libraries: Arc<Libraries>,
	rx: broadcast::Sender<()>,
) {
	if let Err(err) = libraries
		.rx
		.clone()
		.subscribe(|msg| {
			let manager = manager.clone();
			async move {
				match msg {
					LibraryManagerEvent::Load(library) => {
						manager.libraries.load_library(&library).await
					}
					LibraryManagerEvent::Edit(library) => {
						manager.libraries.edit_library(&library).await
					}
					LibraryManagerEvent::InstancesModified(library) => {
						manager.libraries.load_library(&library).await
					}
					LibraryManagerEvent::Delete(library) => {
						manager.libraries.delete_library(&library).await
					}
				}
			}
		})
		.await
	{
		error!("Core may become unstable! `networked_libraries_v2` manager aborted with error: {err:?}");
	}
}

pub use originator::run as originator;
mod originator {
	use super::*;
	use responder::tx as rx;
	use sd_p2p::PeerStatus;

	pub mod tx {
		use super::*;

		pub struct Operations(pub Vec<CRDTOperation>);

		impl Operations {
			// TODO: Per field errors for better error handling
			pub async fn from_stream(
				stream: &mut (impl AsyncRead + Unpin),
			) -> std::io::Result<Self> {
				Ok(Self(
					rmp_serde::from_slice(&decode::buf(stream).await.unwrap()).unwrap(),
				))
			}

			pub fn to_bytes(&self) -> Vec<u8> {
				let Self(args) = self;
				let mut buf = vec![];

				// TODO: Error handling
				encode::buf(&mut buf, &rmp_serde::to_vec_named(&args).unwrap());
				buf
			}
		}
	}

	/// REMEMBER: This only syncs one direction!
	pub async fn run(library_id: Uuid, sync: &Arc<sync::Manager>, p2p: &Arc<super::P2PManager>) {
		let instances = p2p.get_library_service(&library_id).unwrap().get_state();

		// TODO: Deduplicate any duplicate peer ids -> This is an edge case but still
		for instance in instances.values() {
			let PeerStatus::Connected(peer_id) = *instance else {
				continue;
			};

			let sync = sync.clone();
			let p2p = p2p.clone();

			tokio::spawn(async move {
				debug!(
					"Alerting peer '{peer_id:?}' of new sync events for library '{library_id:?}'"
				);

				let mut stream = p2p.manager.stream(peer_id).await.map_err(|_| ()).unwrap(); // TODO: handle providing incorrect peer id

				stream
					.write_all(&Header::Sync(library_id).to_bytes())
					.await
					.unwrap();

				let mut tunnel = Tunnel::initiator(stream).await.unwrap();

				tunnel
					.write_all(&SyncMessage::NewOperations.to_bytes())
					.await
					.unwrap();
				tunnel.flush().await.unwrap();

				while let Ok(rx::MainRequest::GetOperations(args)) =
					rx::MainRequest::from_stream(&mut tunnel).await
				{
					let ops = sync.get_ops(args).await.unwrap();

					tunnel
						.write_all(&tx::Operations(ops).to_bytes())
						.await
						.unwrap();
					tunnel.flush().await.unwrap();
				}
			});
		}
	}
}

pub use responder::run as responder;
mod responder {
	use super::*;
	use originator::tx as rx;

	pub mod tx {
		use serde::{Deserialize, Serialize};

		use super::*;

		#[derive(Serialize, Deserialize)]
		pub enum MainRequest {
			GetOperations(GetOpsArgs),
			Done,
		}

		impl MainRequest {
			// TODO: Per field errors for better error handling
			pub async fn from_stream(
				stream: &mut (impl AsyncRead + Unpin),
			) -> std::io::Result<Self> {
				Ok(
					// TODO: Error handling
					rmp_serde::from_slice(&decode::buf(stream).await.unwrap()).unwrap(),
				)
			}

			pub fn to_bytes(&self) -> Vec<u8> {
				let mut buf = vec![];
				// TODO: Error handling
				encode::buf(&mut buf, &rmp_serde::to_vec_named(&self).unwrap());
				buf
			}
		}
	}

	pub async fn run(stream: &mut (impl AsyncRead + AsyncWrite + Unpin), library: Arc<Library>) {
		let ingest = &library.sync.ingest;

		async fn early_return(stream: &mut (impl AsyncRead + AsyncWrite + Unpin)) {
			// TODO: Proper error returned to remote instead of this.
			// TODO: We can't just abort the connection when the remote is expecting data.
			stream
				.write_all(&tx::MainRequest::Done.to_bytes())
				.await
				.unwrap();
			stream.flush().await.unwrap();
		}

		let Ok(mut rx) = ingest.req_rx.try_lock() else {
			warn!("Rejected sync due to libraries lock being held!");

			return early_return(stream).await;
		};

		use sync::ingest::*;

		ingest.event_tx.send(Event::Notification).await.unwrap();

		while let Some(req) = rx.recv().await {
			const OPS_PER_REQUEST: u32 = 1000;

			let timestamps = match req {
				Request::FinishedIngesting => break,
				Request::Messages { timestamps } => timestamps,
				_ => continue,
			};

			debug!("Getting ops for timestamps {timestamps:?}");

			stream
				.write_all(
					&tx::MainRequest::GetOperations(sync::GetOpsArgs {
						clocks: timestamps,
						count: OPS_PER_REQUEST,
					})
					.to_bytes(),
				)
				.await
				.unwrap();
			stream.flush().await.unwrap();

			let rx::Operations(ops) = rx::Operations::from_stream(stream).await.unwrap();

			ingest
				.event_tx
				.send(Event::Messages(MessagesEvent {
					instance_id: library.sync.instance,
					has_more: ops.len() == OPS_PER_REQUEST as usize,
					messages: ops,
				}))
				.await
				.expect("TODO: Handle ingest channel closed, so we don't loose ops");
		}

		debug!("Sync responder done");

		stream
			.write_all(&tx::MainRequest::Done.to_bytes())
			.await
			.unwrap();
		stream.flush().await.unwrap();
	}
}
