//! Backends with no GPU: [`NullBackend`] validates every call and keeps
//! buffer contents in memory, and [`RecordingBackend`] does the same while
//! logging each call for tests to inspect.

use crate::backend::{BackendError, DrawCall, Geometry, LiveBuffers, RenderBackend, View};
use crate::buffer::{
    IndexBufferDesc, IndexBufferHandle, IndexFormat, RawHandle, VertexBufferDesc,
    VertexBufferHandle,
};
use crate::key::MaterialId;
use crate::texture::{MaterialDesc, TextureDesc, TextureHandle};
use std::collections::HashMap;

#[derive(Debug)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// A generational slot map: freed slots are reused with a bumped generation,
/// so stale handles never resolve.
#[derive(Debug)]
struct Slots<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    live: usize,
}

impl<T> Default for Slots<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            live: 0,
        }
    }
}

impl<T> Slots<T> {
    fn insert(&mut self, value: T) -> RawHandle {
        self.live += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.value = Some(value);
            RawHandle {
                index,
                generation: slot.generation,
            }
        } else {
            let index = u32::try_from(self.slots.len()).expect("more than u32::MAX buffers");
            self.slots.push(Slot {
                generation: 0,
                value: Some(value),
            });
            RawHandle {
                index,
                generation: 0,
            }
        }
    }

    fn get(&self, h: RawHandle) -> Result<&T, BackendError> {
        self.slots
            .get(h.index as usize)
            .filter(|s| s.generation == h.generation)
            .and_then(|s| s.value.as_ref())
            .ok_or_else(|| BackendError::InvalidHandle(format!("{h:?}")))
    }

    fn get_mut(&mut self, h: RawHandle) -> Result<&mut T, BackendError> {
        self.slots
            .get_mut(h.index as usize)
            .filter(|s| s.generation == h.generation)
            .and_then(|s| s.value.as_mut())
            .ok_or_else(|| BackendError::InvalidHandle(format!("{h:?}")))
    }

    fn remove(&mut self, h: RawHandle) -> Result<T, BackendError> {
        let slot = self
            .slots
            .get_mut(h.index as usize)
            .filter(|s| s.generation == h.generation && s.value.is_some())
            .ok_or_else(|| BackendError::InvalidHandle(format!("{h:?}")))?;
        let value = slot.value.take().expect("checked above");
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(h.index);
        self.live -= 1;
        Ok(value)
    }
}

#[derive(Debug)]
struct VertexBuffer {
    desc: VertexBufferDesc,
    data: Vec<u8>,
}

#[derive(Debug)]
struct IndexBuffer {
    desc: IndexBufferDesc,
    data: Vec<u8>,
}

fn initial_bytes(size: u64, initial: Option<&[u8]>) -> Result<Vec<u8>, BackendError> {
    match initial {
        Some(bytes) if bytes.len() as u64 != size => Err(BackendError::InitialDataSize {
            expected: size,
            actual: bytes.len() as u64,
        }),
        Some(bytes) => Ok(bytes.to_vec()),
        None => Ok(vec![0; size as usize]),
    }
}

fn write_bytes(dst: &mut [u8], offset: u64, src: &[u8]) -> Result<(), BackendError> {
    let size = dst.len() as u64;
    let len = src.len() as u64;
    match offset.checked_add(len) {
        Some(end) if end <= size => {
            dst[offset as usize..end as usize].copy_from_slice(src);
            Ok(())
        }
        _ => Err(BackendError::WriteOutOfBounds { offset, len, size }),
    }
}

fn check_range(first: u32, count: u32, available: u32) -> Result<(), BackendError> {
    let end = u64::from(first) + u64::from(count);
    if end > u64::from(available) {
        return Err(BackendError::DrawOutOfBounds {
            first: first.into(),
            end,
            available: available.into(),
        });
    }
    Ok(())
}

/// Validates every call against the resources it holds and draws nothing.
///
/// Catches what would be undefined behaviour or a silent corruption on a
/// real device: stale handles, writes past a buffer's end, draws reading past
/// the vertex or index data (including index values that point outside the
/// vertex buffer), and draws outside a frame.
#[derive(Debug, Default)]
pub struct NullBackend {
    vertex_buffers: Slots<VertexBuffer>,
    index_buffers: Slots<IndexBuffer>,
    textures: Slots<TextureDesc>,
    materials: HashMap<MaterialId, MaterialDesc>,
    in_frame: bool,
    frames: u64,
}

impl NullBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// Completed frames.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn vertex_data(&self, buffer: VertexBufferHandle) -> Result<&[u8], BackendError> {
        Ok(&self.vertex_buffers.get(buffer.0)?.data)
    }

    pub fn index_data(&self, buffer: IndexBufferHandle) -> Result<&[u8], BackendError> {
        Ok(&self.index_buffers.get(buffer.0)?.data)
    }

    /// The material draws with `id` shade with.
    pub fn material(&self, id: MaterialId) -> MaterialDesc {
        self.materials.get(&id).copied().unwrap_or_default()
    }

    pub fn texture_desc(&self, texture: TextureHandle) -> Result<&TextureDesc, BackendError> {
        self.textures.get(texture.0)
    }

    fn validate_draw(&self, call: &DrawCall) -> Result<(), BackendError> {
        if let Some(t) = self.material(call.material).texture {
            self.textures.get(t.0)?;
        }
        let vb = self.vertex_buffers.get(call.vertex_buffer.0)?;
        let vertex_count = vb.desc.vertex_count;
        match call.geometry {
            Geometry::Vertices { first, count } => check_range(first, count, vertex_count),
            Geometry::Indexed {
                buffer,
                first,
                count,
                base_vertex,
            } => {
                let ib = self.index_buffers.get(buffer.0)?;
                check_range(first, count, ib.desc.index_count)?;
                let width = ib.desc.format.size() as usize;
                let start = first as usize * width;
                let bytes = &ib.data[start..start + count as usize * width];
                for chunk in bytes.chunks_exact(width) {
                    let index = match ib.desc.format {
                        IndexFormat::U16 => i64::from(u16::from_le_bytes([chunk[0], chunk[1]])),
                        IndexFormat::U32 => i64::from(u32::from_le_bytes(
                            chunk.try_into().expect("chunk is 4 bytes"),
                        )),
                    };
                    let vertex = index + i64::from(base_vertex);
                    if !(0..i64::from(vertex_count)).contains(&vertex) {
                        return Err(BackendError::DrawOutOfBounds {
                            first: vertex.max(0) as u64,
                            end: vertex.max(0) as u64 + 1,
                            available: vertex_count.into(),
                        });
                    }
                }
                Ok(())
            }
        }
    }
}

impl RenderBackend for NullBackend {
    fn create_vertex_buffer(
        &mut self,
        desc: &VertexBufferDesc,
        initial: Option<&[u8]>,
    ) -> Result<VertexBufferHandle, BackendError> {
        let data = initial_bytes(desc.size_in_bytes(), initial)?;
        let desc = desc.clone();
        Ok(VertexBufferHandle(
            self.vertex_buffers.insert(VertexBuffer { desc, data }),
        ))
    }

    fn create_index_buffer(
        &mut self,
        desc: &IndexBufferDesc,
        initial: Option<&[u8]>,
    ) -> Result<IndexBufferHandle, BackendError> {
        let data = initial_bytes(desc.size_in_bytes(), initial)?;
        Ok(IndexBufferHandle(
            self.index_buffers.insert(IndexBuffer { desc: *desc, data }),
        ))
    }

    fn write_vertex_buffer(
        &mut self,
        buffer: VertexBufferHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<(), BackendError> {
        write_bytes(
            &mut self.vertex_buffers.get_mut(buffer.0)?.data,
            offset,
            data,
        )
    }

    fn write_index_buffer(
        &mut self,
        buffer: IndexBufferHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<(), BackendError> {
        write_bytes(
            &mut self.index_buffers.get_mut(buffer.0)?.data,
            offset,
            data,
        )
    }

    fn destroy_vertex_buffer(&mut self, buffer: VertexBufferHandle) -> Result<(), BackendError> {
        self.vertex_buffers.remove(buffer.0).map(drop)
    }

    fn destroy_index_buffer(&mut self, buffer: IndexBufferHandle) -> Result<(), BackendError> {
        self.index_buffers.remove(buffer.0).map(drop)
    }

    fn create_texture(
        &mut self,
        desc: &TextureDesc,
        mips: &[&[u8]],
    ) -> Result<TextureHandle, BackendError> {
        desc.validate(mips)?;
        Ok(TextureHandle(self.textures.insert(*desc)))
    }

    fn destroy_texture(&mut self, texture: TextureHandle) -> Result<(), BackendError> {
        self.textures.remove(texture.0).map(drop)
    }

    fn set_material(&mut self, id: MaterialId, desc: &MaterialDesc) -> Result<(), BackendError> {
        if let Some(t) = desc.texture {
            self.textures.get(t.0)?;
        }
        self.materials.insert(id, *desc);
        Ok(())
    }

    fn live_buffers(&self) -> LiveBuffers {
        LiveBuffers {
            vertex: self.vertex_buffers.live,
            index: self.index_buffers.live,
            texture: self.textures.live,
        }
    }

    fn begin_frame(&mut self, _view: &View) -> Result<(), BackendError> {
        if self.in_frame {
            return Err(BackendError::FrameAlreadyOpen);
        }
        self.in_frame = true;
        Ok(())
    }

    fn draw(&mut self, call: &DrawCall) -> Result<(), BackendError> {
        if !self.in_frame {
            return Err(BackendError::NotInFrame("draw"));
        }
        self.validate_draw(call)
    }

    fn end_frame(&mut self) -> Result<(), BackendError> {
        if !self.in_frame {
            return Err(BackendError::NotInFrame("end_frame"));
        }
        self.in_frame = false;
        self.frames += 1;
        Ok(())
    }
}

/// One call a [`RecordingBackend`] accepted.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    CreateVertexBuffer {
        handle: VertexBufferHandle,
        desc: VertexBufferDesc,
    },
    CreateIndexBuffer {
        handle: IndexBufferHandle,
        desc: IndexBufferDesc,
    },
    WriteVertexBuffer {
        handle: VertexBufferHandle,
        offset: u64,
        len: usize,
    },
    WriteIndexBuffer {
        handle: IndexBufferHandle,
        offset: u64,
        len: usize,
    },
    DestroyVertexBuffer(VertexBufferHandle),
    DestroyIndexBuffer(IndexBufferHandle),
    CreateTexture {
        handle: TextureHandle,
        desc: TextureDesc,
        mips: usize,
    },
    DestroyTexture(TextureHandle),
    SetMaterial(MaterialId, MaterialDesc),
    BeginFrame(View),
    Draw(DrawCall),
    EndFrame,
}

/// A [`NullBackend`] that also logs every successful call.
///
/// Rejected calls are not logged, so the log is exactly what a real backend
/// would have been asked to do.
#[derive(Debug, Default)]
pub struct RecordingBackend {
    inner: NullBackend,
    commands: Vec<Command>,
}

impl RecordingBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// Returns and clears the log.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    /// The draws in the log, in order.
    pub fn draws(&self) -> impl Iterator<Item = &DrawCall> {
        self.commands.iter().filter_map(|c| match c {
            Command::Draw(call) => Some(call),
            _ => None,
        })
    }

    pub fn inner(&self) -> &NullBackend {
        &self.inner
    }

    fn log<T>(
        &mut self,
        result: Result<T, BackendError>,
        command: impl FnOnce(&T) -> Command,
    ) -> Result<T, BackendError> {
        if let Ok(value) = &result {
            self.commands.push(command(value));
        }
        result
    }
}

impl RenderBackend for RecordingBackend {
    fn create_vertex_buffer(
        &mut self,
        desc: &VertexBufferDesc,
        initial: Option<&[u8]>,
    ) -> Result<VertexBufferHandle, BackendError> {
        let result = self.inner.create_vertex_buffer(desc, initial);
        self.log(result, |&handle| Command::CreateVertexBuffer {
            handle,
            desc: desc.clone(),
        })
    }

    fn create_index_buffer(
        &mut self,
        desc: &IndexBufferDesc,
        initial: Option<&[u8]>,
    ) -> Result<IndexBufferHandle, BackendError> {
        let result = self.inner.create_index_buffer(desc, initial);
        self.log(result, |&handle| Command::CreateIndexBuffer {
            handle,
            desc: *desc,
        })
    }

    fn write_vertex_buffer(
        &mut self,
        handle: VertexBufferHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<(), BackendError> {
        let result = self.inner.write_vertex_buffer(handle, offset, data);
        self.log(result, |()| Command::WriteVertexBuffer {
            handle,
            offset,
            len: data.len(),
        })
    }

    fn write_index_buffer(
        &mut self,
        handle: IndexBufferHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<(), BackendError> {
        let result = self.inner.write_index_buffer(handle, offset, data);
        self.log(result, |()| Command::WriteIndexBuffer {
            handle,
            offset,
            len: data.len(),
        })
    }

    fn destroy_vertex_buffer(&mut self, handle: VertexBufferHandle) -> Result<(), BackendError> {
        let result = self.inner.destroy_vertex_buffer(handle);
        self.log(result, |()| Command::DestroyVertexBuffer(handle))
    }

    fn destroy_index_buffer(&mut self, handle: IndexBufferHandle) -> Result<(), BackendError> {
        let result = self.inner.destroy_index_buffer(handle);
        self.log(result, |()| Command::DestroyIndexBuffer(handle))
    }

    fn create_texture(
        &mut self,
        desc: &TextureDesc,
        mips: &[&[u8]],
    ) -> Result<TextureHandle, BackendError> {
        let result = self.inner.create_texture(desc, mips);
        self.log(result, |&handle| Command::CreateTexture {
            handle,
            desc: *desc,
            mips: mips.len(),
        })
    }

    fn destroy_texture(&mut self, handle: TextureHandle) -> Result<(), BackendError> {
        let result = self.inner.destroy_texture(handle);
        self.log(result, |()| Command::DestroyTexture(handle))
    }

    fn set_material(&mut self, id: MaterialId, desc: &MaterialDesc) -> Result<(), BackendError> {
        let result = self.inner.set_material(id, desc);
        self.log(result, |()| Command::SetMaterial(id, *desc))
    }

    fn live_buffers(&self) -> LiveBuffers {
        self.inner.live_buffers()
    }

    fn begin_frame(&mut self, view: &View) -> Result<(), BackendError> {
        let result = self.inner.begin_frame(view);
        self.log(result, |()| Command::BeginFrame(*view))
    }

    fn draw(&mut self, call: &DrawCall) -> Result<(), BackendError> {
        let result = self.inner.draw(call);
        self.log(result, |()| Command::Draw(*call))
    }

    fn end_frame(&mut self) -> Result<(), BackendError> {
        let result = self.inner.end_frame();
        self.log(result, |()| Command::EndFrame)
    }
}
