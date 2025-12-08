# **PipeWire Windowing System - Complete Feature Set**

## **Core Architecture**

### **Windows as Multi-Port Video Sources**
- Apps create PipeWire source nodes with multiple output ports
- Each port streams at its own framerate (UI @ 30fps, video @ 60fps, canvas @ 144fps)
- Stream parameters on each port define: position rect, z-index, layer properties
- Dynamic port creation/removal for video elements, canvases, etc.
- Compositor discovers nodes, composites all ports together

### **Unified Protocol**
- Single PipeWire protocol replaces Wayland + XDG portal stack
- No protocol proliferation - windows, audio, video, input all use PipeWire
- Version management simplified - update/downgrade PipeWire instead of entire compositor
- Eliminates per-compositor XDG implementation bugs and incompatibilities

---

## **Window Management**

### **State Communication**
- Stream parameters handle all window state (position, size, scale, fullscreen, etc.)
- Custom SPA structs for window-specific metadata
- Compositor assigns initial region on first connection
- Apps receive callbacks and begin rendering to assigned region

### **No Popup Windows**
- Cleaner UI model - users hate popup proliferation
- Apps create additional window nodes for dialogs/related windows
- Optional parent_id metadata for window relationships
- Apps with focus can transfer focus to owned windows via node ID

### **Compositor-Side Caching**
- Compositor caches last buffer from each window/port
- Window movement uses cached buffers (no app involvement)
- Resize handled via two-phase approach or async rendering
- Same smoothness as Wayland compositors, potentially faster due to PipeWire's real-time infrastructure

---

## **Input Management**

### **Focus Groups**
- Input devices assigned to focus groups
- When one device in group changes focus (e.g., alt-tab), all devices in group move together
- Devices without groups move all focus except grouped devices
- Manual device assignment available for power users

### **Input Routing Model**
- Apps expose input sink nodes
- Compositor creates input source nodes from hardware (libinput/evdev)
- Compositor routes sources to sinks based on focus policy
- Explicit, visible routing - no hidden focus stealing

---

## **Revolutionary Capabilities**

### **Zero-Copy Throughout**
- DMA-BUF from app GPU buffers to display
- Multi-port streams enable zero-copy video playback
- Video decoder → port → compositor → display (no intermediate copies)
- Direct app-to-hardware paths when appropriate

### **Trivial Screen Sharing & Recording**
```bash
# Share specific video element
pw-link app.video_layer recording.sink

# Share entire window
pw-link compositor.window_123 video_conference.sink

# Record workspace
pw-link compositor.output recording.file
```
- No special APIs needed - just PipeWire graph connections
- Per-layer sharing (share just the video element, not UI)
- Live reconfiguration without app restarts

### **Visual Desktop Management**
- Use qpwgraph to see all windows, audio, input as nodes
- Drag-and-drop routing for power users (optional)
- Complete visibility into desktop stream topology
- Live input device reassignment
- Debugging and monitoring built-in

### **Mixed-Framerate Compositing**
- Each port renders at its natural framerate
- UI updates on-demand (efficient)
- Videos play at their native framerate (24/30/60fps)
- Games render at high refresh rates (144fps+)
- Compositor pulls latest frame from each port at display refresh rate
- Natural optimization - compositor only recomposites changed layers

### **Efficient Static Content**
- Ports can send frames on-demand (UI) or continuously (games/video)
- Compositor reuses cached buffers when no new frame available
- Apps control rendering frequency per-port

---

## **Security & Permissions**

### **Default-Deny Policy**
- Apps cannot connect without permission (via WirePlumber)
- Stream-state permission model - reactive enforcement
- Privileged tool allowlist (compositor, qpwgraph, etc.)
- Automatic sandboxing for legacy/untrusted apps

### **Visible Security Model**
- All routing visible in graph tools
- Clear indication of which apps have access to what
- Audit trail of connections
- Consistent permission model across windows, audio, input, video

---

## **Developer Experience**

### **Single Unified API**
- One PipeWire API for windows, input, audio, video
- No separate xdg-desktop-portal complexity
- Direct buffer management with DMA-BUF
- Screen sharing = just connect nodes (no special protocols)

### **Per-Layer Control**
```rust
// Create window with UI layer
let window = create_window_node();
let ui_port = window.add_output_port("ui_layer");

// Add video player dynamically
let video_port = window.add_output_port("video_layer");
video_port.set_params(rect: (100, 50, 640, 480), z: 5);
decoder.connect(video_port);

// Remove when done
window.remove_port(video_port);
```

### **Simplified Compositor Development**
Compositor responsibilities reduced to:
- Discover window nodes and their ports
- Read stream params for layer positioning
- Composite port buffers according to z-index
- Route input based on focus policy
- Manage layout (tiling/floating/etc.)

Hard problems handled by PipeWire:
- Buffer management and negotiation
- Format negotiation
- Multi-client synchronization
- DMA-BUF handling
- Low-latency streaming
- Permission/policy infrastructure

---

## **Performance Advantages**

### **Real-Time Infrastructure**
- Built on PipeWire's pro-audio low-latency foundation
- Quantum-aware compositing for predictable timing
- Better A/V sync for video playback windows
- Potentially smoother window operations than Wayland

### **Automatic Optimization**
- Zero-copy paths throughout
- Efficient damage tracking per-port
- Hardware acceleration via DMA-BUF
- Minimal compositor overhead

### **Rate Matching**
- Fast producers (games) → slow consumers (display) handled automatically
- Buffer queuing and frame dropping managed by PipeWire
- Apps can query timing feedback for optimization

---

## **Compatibility & Adoption**

### **Wayland Compatibility Layer**
- Translation layer for existing Wayland apps
- Apps run without modification
- Wayland protocol → PipeWire nodes translation
- Gradual migration path: legacy apps translated, new apps native

### **Implementation Stack**
- Rust + DRM + GBM for compositor
- Custom SPA structs for window management metadata
- WirePlumber for policy/permissions
- Standard PipeWire buffer negotiation
- Built on proven technologies (PipeWire, DMA-BUF standards, Flatpak security)

---

## **Power User Features**

### **Complete Control**
- Visual graph manipulation (optional via qpwgraph)
- Manual input routing overrides
- Per-device focus group assignment
- Scriptable graph changes for automation
- Direct buffer control for performance tuning

### **Debugging & Monitoring**
- See all desktop streams in real-time
- Monitor framerate per window/port
- Track buffer flow and timing
- Identify performance bottlenecks
- Complete connection history

---

## **The Core Vision**

Replace complex desktop protocol stacks with a **simple, unified PipeWire graph** where:

- **Everything is visible** - windows, audio, video, input all in one topology
- **Complex features become simple connections** - screen sharing, recording, routing
- **Security is built-in** - default-deny with visible access patterns  
- **Performance is automatic** - real-time infrastructure, zero-copy, efficient caching
- **Maintenance is centralized** - PipeWire team handles hard problems
- **Development is simplified** - one API, consistent behavior, clear responsibilities

A fundamental simplification that makes the desktop simultaneously **more powerful, more transparent, and easier to understand**.