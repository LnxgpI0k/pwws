type SavedState =
   (Vec<(CrtcHandle, CrtcInfo)>, Vec<(ConnectorHandle, Option<CrtcHandle>)>);

fn get_current_state(card: &Card) -> crate::error::CompositorResult<SavedState> {
   let res =
      card.resource_handles().map_err(|e| CompositorError::ResourcesError(e))?;
   let mut saved_crtcs = Vec::new();
   for &crtc_handle in res.crtcs() {
      let crtc_info =
         card
            .get_crtc(crtc_handle)
            .map_err(|e| CompositorError::GetCrtcInfo(crtc_handle, e))?;
      saved_crtcs.push((crtc_handle, crtc_info));
   }
   let mut saved_connectors = Vec::new();
   for &conn_handle in res.connectors() {
      let conn_info =
         card
            .get_connector(conn_handle, false)
            .map_err(|e| CompositorError::GetConnectorInfo(conn_handle, e))?;
      let mut crtc_handle = None;
      if let Some(encoder_handle) = conn_info.current_encoder() {
         let encoder_info =
            card
               .get_encoder(encoder_handle)
               .map_err(|e| CompositorError::GetEncoderInfo(encoder_handle, e))?;
         crtc_handle = encoder_info.crtc();
      }
      saved_connectors.push((conn_handle, crtc_handle));
   }
   Ok((saved_crtcs, saved_connectors))
}

fn reset_tty() {
   if let Ok(tty) = std::fs::File::open("/dev/tty") {
      const VT_GETSTATE: u64 = 0x5603;
      const VT_ACTIVATE: u64 = 0x5606;

      #[repr(C)]
      struct VtStat {
         v_active: u16,
         v_signal: u16,
         v_state: u16,
      }

      let mut vt_stat = VtStat {
         v_active: 0,
         v_signal: 0,
         v_state: 0,
      };
      unsafe {
         libc::ioctl(tty.as_raw_fd(), VT_GETSTATE, &mut vt_stat);
         libc::ioctl(tty.as_raw_fd(), VT_ACTIVATE, vt_stat.v_active as i32);
      }
   }
}

fn restore_state(card: &Card, state: SavedState) {
   for (crtc_handle, crtc_info) in state.0 {
      let connectors =
         state
            .1
            .iter()
            .flat_map(
               |(conn_handle, conn_crtc_handle)| if (*conn_crtc_handle)? ==
                  crtc_handle {
                  Some(*conn_handle)
               } else {
                  None
               },
            )
            .collect::<Vec<_>>();
      card
         .set_crtc(
            crtc_handle,
            None,
            crtc_info.position(),
            &connectors,
            crtc_info.mode(),
         )
         .expect("Failed to restore CRTC");
   }
   reset_tty();
}

fn start_signal_thread(card: &Card) {
   let saved_state = get_current_state(&card).expect("Failed to save state");
   let mut signals =
      signal_hook::iterator::Signals::new(
         &[signal_hook::consts::SIGTERM],
      ).expect("Failed to get signals");
   {
      let card = Card::from_raw(card.as_fd().as_raw_fd());
      let saved_state = saved_state.clone();
      std::thread::spawn(move || {
         for _ in signals.forever() {
            restore_state(&card, saved_state);
            break;
         }
      });
   }
}
