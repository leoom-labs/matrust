use eframe::egui;

/// Reject malformed wheel deltas before egui performs vector arithmetic.
pub(super) fn sanitize(input: &mut egui::RawInput) {
    input.events.retain(
        |event| !matches!(event, egui::Event::MouseWheel { delta, .. } if !delta.is_finite()),
    );
}
