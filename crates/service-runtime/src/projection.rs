//! Projection delivery semantics.

use crate::{BoxFuture, EventEnvelope, StreamId, StreamVersion, VerifiedAuthContext};

/// Delivery guarantee declared for a projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionDelivery {
    /// Execution returns only after the appended version is visible to projection readers.
    ReadYourWrites,
    /// Execution returns after projection work has been accepted for eventual delivery.
    Eventual,
}

/// The exact stream position a projection must reach.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionTarget {
    stream: StreamId,
    through: StreamVersion,
}

impl ProjectionTarget {
    /// Creates a projection target.
    pub fn new(stream: StreamId, through: StreamVersion) -> Self {
        Self { stream, through }
    }

    /// Returns the projected stream.
    pub fn stream(&self) -> &StreamId {
        &self.stream
    }

    /// Returns the version that must become visible.
    pub const fn through(&self) -> StreamVersion {
        self.through
    }
}

/// Visibility established before execution returns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionVisibility {
    /// Work was accepted and may still be converging.
    Scheduled,
    /// The target version was confirmed visible.
    Visible,
}

/// Projection result reported by aggregate execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionOutcome {
    target: ProjectionTarget,
    visibility: ProjectionVisibility,
}

impl ProjectionOutcome {
    /// Creates a projection outcome.
    pub fn new(target: ProjectionTarget, visibility: ProjectionVisibility) -> Self {
        Self { target, visibility }
    }

    /// Returns the target position.
    pub fn target(&self) -> &ProjectionTarget {
        &self.target
    }

    /// Returns the visibility established before execution returned.
    pub const fn visibility(&self) -> ProjectionVisibility {
        self.visibility
    }
}

/// Port for reducing committed envelopes into read models.
///
/// `project` must be idempotent by event identity. The runtime may submit replayed append receipts
/// after a crash between append and projection.
pub trait ProjectionSink<S, E>: Send {
    /// Adapter-specific projection failure.
    type Error;

    /// Accepts committed events and the corresponding reduced aggregate state.
    fn project<'a>(
        &'a mut self,
        context: &'a VerifiedAuthContext,
        target: &'a ProjectionTarget,
        state: &'a S,
        events: &'a [EventEnvelope<E>],
    ) -> BoxFuture<'a, Result<(), Self::Error>>;

    /// Asynchronously waits until readers observe at least the target position.
    fn wait_until_visible<'a>(
        &'a mut self,
        target: &'a ProjectionTarget,
    ) -> BoxFuture<'a, Result<(), Self::Error>>;
}

pub(crate) async fn deliver<S, E, P>(
    sink: &mut P,
    context: &VerifiedAuthContext,
    delivery: ProjectionDelivery,
    target: ProjectionTarget,
    state: &S,
    events: &[EventEnvelope<E>],
) -> Result<ProjectionOutcome, P::Error>
where
    P: ProjectionSink<S, E> + ?Sized,
{
    sink.project(context, &target, state, events).await?;
    let visibility = match delivery {
        ProjectionDelivery::ReadYourWrites => {
            sink.wait_until_visible(&target).await?;
            ProjectionVisibility::Visible
        }
        ProjectionDelivery::Eventual => ProjectionVisibility::Scheduled,
    };
    Ok(ProjectionOutcome::new(target, visibility))
}
