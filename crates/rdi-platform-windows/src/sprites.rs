//! Instanced D3D11 rendering of immutable, D2D-prepared icon artwork.

use crate::shader;
use rdi_core::{DesktopError, Effect, IconFrame, IconId, Point, ShaderPipeline, ShaderProgram};
use std::collections::HashMap;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, D3D_PRIMITIVE_TOPOLOGY_LINELIST};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGISurface;
use windows::core::{Interface, s};

pub(crate) fn gpu_error(error: windows::core::Error) -> DesktopError {
    DesktopError::InvalidEffect(format!("D3D11: {error}"))
}

pub(crate) struct Sprite {
    pub id: IconId,
    pub origin: Point,
    pub destination: Point,
    pub region: [f32; 4],
    pub anchor: [f32; 2],
    pub geometry: [f32; 4],
    pub effect: Option<Effect>,
}

pub(crate) struct CaptureTarget {
    pub surface: IDXGISurface,
    target: ID3D11Texture2D,
    staging: ID3D11Texture2D,
    context: ID3D11DeviceContext,
    width: u32,
    height: u32,
}

impl CaptureTarget {
    pub fn new(device: &ID3D11Device, width: u32, height: u32) -> Result<Self, DesktopError> {
        let target = texture(device, width, height)?;
        let surface = target.cast().map_err(gpu_error)?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        let mut staging = None;
        // SAFETY: target belongs to this device; matching CPU-readable staging descriptor and valid output slots.
        let context = unsafe {
            target.GetDesc(&mut desc);
            desc.BindFlags = 0;
            desc.Usage = D3D11_USAGE_STAGING;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            device.CreateTexture2D(&desc, None, Some(&mut staging)).map_err(gpu_error)?;
            device.GetImmediateContext().map_err(gpu_error)?
        };
        Ok(Self { surface, target, staging: staging.ok_or_else(|| DesktopError::InvalidEffect("missing staging texture".into()))?, context, width, height })
    }

    pub fn read(&self, seconds: f64) -> Result<rdi_core::CapturedFrame, DesktopError> {
        let mut pixels = vec![0; self.width as usize * self.height as usize * 4];
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: matching resources on the owning worker; Map synchronizes CopyResource. Copy each valid row before Unmap.
        unsafe {
            self.context.CopyResource(&self.staging, &self.target);
            self.context.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).map_err(gpu_error)?;
            let stride = self.width as usize * 4;
            for row in 0..self.height as usize {
                let source = std::slice::from_raw_parts(mapped.pData.cast::<u8>().add(row * mapped.RowPitch as usize), stride);
                pixels[row * stride..(row + 1) * stride].copy_from_slice(source);
            }
            self.context.Unmap(&self.staging, 0);
        }
        Ok(rdi_core::CapturedFrame { width: self.width, height: self.height, seconds, pixels })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Instance {
    rect: [f32; 4],
    region: [f32; 4],
    timing: [f32; 4],
    params: [f32; 4],
    geometry: [f32; 4],
    travel: [f32; 4],
    body: [f32; 4],
    label: [f32; 4],
}

pub(crate) struct SpriteRenderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext1,
    programs: Vec<GpuProgram>,
    program_indices: Vec<usize>,
    atlas: ID3D11ShaderResourceView,
    atlas_size: [f32; 2],
    sampler: ID3D11SamplerState,
    blend: ID3D11BlendState,
    rasterizer: ID3D11RasterizerState,
    constants: ID3D11Buffer,
    values: ID3D11Buffer,
    instances: ID3D11Buffer,
    sprites: Vec<Sprite>,
    lookup: HashMap<IconId, usize>,
    data: Vec<Instance>,
}

struct GpuProgram {
    execution: Option<GpuExecution>,
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    layout: ID3D11InputLayout,
}

struct GpuExecution {
    targets: Vec<(ID3D11RenderTargetView, ID3D11ShaderResourceView)>,
    passes: Vec<(GpuProgram, ID3D11BlendState)>,
}

pub(crate) fn texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<ID3D11Texture2D, DesktopError> {
    let descriptor = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET).0 as u32,
        ..Default::default()
    };
    let mut result = None;
    // SAFETY: the descriptor defines one valid BGRA texture; the output slot lives through the call.
    unsafe { device.CreateTexture2D(&descriptor, None, Some(&mut result)) }.map_err(gpu_error)?;
    result.ok_or_else(|| DesktopError::InvalidEffect("missing texture".into()))
}

impl SpriteRenderer {
    #[cfg(test)]
    pub(crate) fn new(
        device: &ID3D11Device,
        atlas: &ID3D11Texture2D,
        sprites: Vec<Sprite>,
    ) -> Result<Self, DesktopError> {
        let layers = sprites.iter().map(|sprite| [sprite.region; 2]).collect();
        Self::new_layered(device, atlas, sprites, layers)
    }

    pub(crate) fn new_layered(
        device: &ID3D11Device,
        atlas: &ID3D11Texture2D,
        sprites: Vec<Sprite>,
        layers: Vec<[[f32; 4]; 2]>,
    ) -> Result<Self, DesktopError> {
        if layers.len() != sprites.len() {
            return Err(DesktopError::InvalidEffect("artwork layer count mismatch".into()));
        }
        let identity = shader::compile(shader::BuiltinShader::Identity)?;
        let mut atlas_view = None;
        let mut sampler = None;
        let mut blend = None;
        let mut rasterizer = None;
        let mut blend_desc = D3D11_BLEND_DESC::default();
        blend_desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            SrcBlend: D3D11_BLEND_ONE,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        // SAFETY: all bytecode, descriptors and output slots outlive the calls; objects share this device.
        let context = unsafe {
            device
                .CreateShaderResourceView(atlas, None, Some(&mut atlas_view))
                .map_err(gpu_error)?;
            device
                .CreateSamplerState(
                    &D3D11_SAMPLER_DESC {
                        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                        MaxLOD: f32::MAX,
                        ComparisonFunc: D3D11_COMPARISON_NEVER,
                        ..Default::default()
                    },
                    Some(&mut sampler),
                )
                .map_err(gpu_error)?;
            device
                .CreateBlendState(&blend_desc, Some(&mut blend))
                .map_err(gpu_error)?;
            device
                .CreateRasterizerState(
                    &D3D11_RASTERIZER_DESC {
                        FillMode: D3D11_FILL_SOLID,
                        CullMode: D3D11_CULL_NONE,
                        DepthClipEnable: true.into(),
                        ScissorEnable: true.into(),
                        ..Default::default()
                    },
                    Some(&mut rasterizer),
                )
                .map_err(gpu_error)?;
            device
                .GetImmediateContext()
                .map_err(gpu_error)?
                .cast::<ID3D11DeviceContext1>()
                .map_err(gpu_error)?
        };
        let mut programs = vec![gpu_program(device, &identity)?];
        let mut bytecodes = vec![identity];
        let mut program_indices = Vec::with_capacity(sprites.len());
        let mut target_bytes = 0u64;
        for sprite in &sprites {
            if let Some(effect) = &sprite.effect {
                effect.validate()?;
                let index = match bytecodes
                    .iter()
                    .position(|program| program == &effect.shader)
                {
                    Some(index) => index,
                    None => {
                        target_bytes += effect.shader.execution.as_ref().map_or(0, |execution| execution.target_bytes());
                        if target_bytes > 256 * 1024 * 1024 {
                            return Err(DesktopError::InvalidEffect("session intermediate textures exceed 256 MiB".into()));
                        }
                        programs.push(gpu_program(device, &effect.shader)?);
                        bytecodes.push(effect.shader.clone());
                        bytecodes.len() - 1
                    }
                };
                program_indices.push(index);
            } else {
                program_indices.push(0);
            }
        }
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: desc is a valid writable descriptor.
        unsafe { atlas.GetDesc(&mut desc) };
        let lookup = sprites
            .iter()
            .enumerate()
            .map(|(index, sprite)| (sprite.id.clone(), index))
            .collect();
        let data = sprites
            .iter()
            .zip(layers)
            .map(|(sprite, layers)| Instance {
                body: layers[0],
                label: layers[1],
                rect: [
                    sprite.origin.x as f32 - sprite.anchor[0],
                    sprite.origin.y as f32 - sprite.anchor[1],
                    sprite.region[2],
                    sprite.region[3],
                ],
                region: sprite.region,
                geometry: sprite.geometry,
                travel: [
                    0.0,
                    0.0,
                    sprite.destination.x as f32 - sprite.origin.x as f32,
                    sprite.destination.y as f32 - sprite.origin.y as f32,
                ],
                timing: [0.0; 4],
                params: sprite
                    .effect
                    .as_ref()
                    .map_or([0.0; 4], |effect| effect.params),
            })
            .collect();
        Ok(Self {
            device: device.clone(),
            context,
            atlas: atlas_view.unwrap(),
            sampler: sampler.unwrap(),
            blend: blend.unwrap(),
            rasterizer: rasterizer.unwrap(),
            constants: buffer(device, 32, D3D11_BIND_CONSTANT_BUFFER)?,
            values: buffer(device, 64, D3D11_BIND_CONSTANT_BUFFER)?,
            instances: buffer(
                device,
                (sprites.len().max(1) * size_of::<Instance>()) as u32,
                D3D11_BIND_VERTEX_BUFFER,
            )?,
            atlas_size: [desc.Width as f32, desc.Height as f32],
            programs,
            program_indices,
            sprites,
            lookup,
            data,
        })
    }

    pub(crate) fn render_region(
        &mut self,
        surface: &IDXGISurface,
        offset: [f32; 2],
        update: RECT,
        origin: Point,
        frame: &[IconFrame],
        clean: bool,
    ) -> Result<(), DesktopError> {
        let target: ID3D11Texture2D = surface.cast().map_err(gpu_error)?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: target owns the resource; desc is a valid output slot.
        unsafe { target.GetDesc(&mut desc) };
        if update.left < 0
            || update.top < 0
            || update.right <= update.left
            || update.bottom <= update.top
            || update.right as u32 > desc.Width
            || update.bottom as u32 > desc.Height
        {
            return Err(DesktopError::InvalidEffect(
                "invalid composition update rectangle".into(),
            ));
        }
        for entry in frame {
            if let Some(&index) = self.lookup.get(&entry.id) {
                let sprite = &self.sprites[index];
                self.data[index].rect[0] = (entry.position.x - origin.x) as f32 - sprite.anchor[0];
                self.data[index].rect[1] = (entry.position.y - origin.y) as f32 - sprite.anchor[1];
                self.data[index].travel = [
                    sprite.origin.x as f32 - entry.position.x as f32,
                    sprite.origin.y as f32 - entry.position.y as f32,
                    sprite.destination.x as f32 - entry.position.x as f32,
                    sprite.destination.y as f32 - entry.position.y as f32,
                ];
                self.data[index].timing = [
                    entry.elapsed_seconds,
                    entry.progress,
                    if clean {
                        0.0
                    } else {
                        sprite
                            .effect
                            .as_ref()
                            .map_or(0.0, |effect| effect.strength(entry.progress))
                    },
                    sprite.effect.as_ref().map_or(0.0, |effect| effect.seed),
                ];
            }
        }
        if clean {
            for instance in &mut self.data {
                instance.timing[2] = 0.0;
            }
        }
        let mut view = None;
        // SAFETY: target belongs to this device; descriptors and output slots are valid.
        unsafe {
            self.device
                .CreateRenderTargetView(&target, None, Some(&mut view))
                .map_err(gpu_error)?;
        }
        let view = view.unwrap();
        let constants = [
            desc.Width as f32,
            desc.Height as f32,
            self.atlas_size[0],
            self.atlas_size[1],
            offset[0],
            offset[1],
            0.0,
            0.0,
        ];
        upload(&self.context, &self.constants, &constants)?;
        if !self.data.is_empty() {
            upload(&self.context, &self.instances, &self.data)?;
        }
        // SAFETY: every bound object belongs to this device. Buffers hold all instances and draws stay within them.
        unsafe {
            self.context
                .OMSetRenderTargets(Some(&[Some(view.clone())]), None);
            self.context.ClearView(&view, &[0.0; 4], Some(&[update]));
            self.context.OMSetBlendState(&self.blend, None, u32::MAX);
            self.context.OMSetDepthStencilState(None, 0);
            self.context.RSSetState(&self.rasterizer);
            self.context.RSSetScissorRects(Some(&[update]));
            self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                Width: desc.Width as f32,
                Height: desc.Height as f32,
                MaxDepth: 1.0,
                ..Default::default()
            }]));
            self.context.IASetInputLayout(&self.programs[0].layout);
            self.context
                .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            let stride = size_of::<Instance>() as u32;
            let byte_offset = 0u32;
            self.context.IASetVertexBuffers(
                0,
                1,
                Some([Some(self.instances.clone())].as_ptr()),
                Some(&stride),
                Some(&byte_offset),
            );
            self.context.VSSetShader(&self.programs[0].vertex, None);
            self.context.GSSetShader(None, None);
            self.context.HSSetShader(None, None);
            self.context.DSSetShader(None, None);
            self.context
                .VSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            self.context
                .PSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            self.context.VSSetConstantBuffers(1, Some(&[Some(self.values.clone())]));
            self.context.PSSetConstantBuffers(1, Some(&[Some(self.values.clone())]));
            self.context
                .PSSetShaderResources(0, Some(&[Some(self.atlas.clone())]));
            self.context
                .PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.context
                .VSSetShaderResources(0, Some(&[Some(self.atlas.clone())]));
            self.context
                .VSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            let mut first = 0;
            while first < self.data.len() {
                let particles =
                    |index: usize| {
                        self.sprites[index].effect.as_ref().is_some_and(|effect| {
                            effect.shader.pipeline != ShaderPipeline::Sprite
                        }) && self.data[index].timing[2] > 0.0
                    };
                if particles(first) {
                    let program = &self.programs[self.program_indices[first]];
                    if let Some(execution) = &program.execution {
                        self.render_execution(first, execution, &view, constants, update)?;
                        first += 1;
                        continue;
                    }
                    self.context.IASetInputLayout(&program.layout);
                    self.context.VSSetShader(&program.vertex, None);
                    self.context.PSSetShader(&program.pixel, None);
                    let instance = &self.data[first];
                    let columns = (instance.region[2] / instance.params[0])
                        .ceil()
                        .clamp(1.0, 64.0) as u32;
                    let rows = (instance.region[3] / instance.params[0])
                        .ceil()
                        .clamp(1.0, 64.0) as u32;
                    let vertices = columns * rows * 6;
                    self.context.DrawInstanced(vertices, 1, 0, first as u32);
                    first += 1;
                    continue;
                }
                let effective_program = |index: usize| {
                    if self.data[index].timing[2] <= 0.0 {
                        0
                    } else {
                        self.program_indices[index]
                    }
                };
                let program = effective_program(first);
                let mut end = first + 1;
                while end < self.data.len() && !particles(end) && effective_program(end) == program
                {
                    end += 1;
                }
                self.context
                    .IASetInputLayout(&self.programs[program].layout);
                self.context
                    .VSSetShader(&self.programs[program].vertex, None);
                self.context
                    .PSSetShader(&self.programs[program].pixel, None);
                self.context
                    .DrawInstanced(6, (end - first) as u32, 0, first as u32);
                first = end;
            }
            self.context.PSSetShaderResources(0, Some(&[None]));
            self.context.VSSetShaderResources(0, Some(&[None]));
            self.context.OMSetRenderTargets(None, None);
        }
        Ok(())
    }

    fn render_execution(&self, index: usize, gpu: &GpuExecution, scene: &ID3D11RenderTargetView,
        scene_constants: [f32; 8], update: RECT) -> Result<(), DesktopError> {
        let effect = self.sprites[index].effect.as_ref().unwrap();
        let execution = effect.shader.execution.as_ref().unwrap();
        let values = effect.parameter_values();
        upload(&self.context, &self.values, &values)?;
        for (pass, (program, blend)) in execution.passes.iter().zip(&gpu.passes) {
            let (view, constants, clip) = if let Some(target) = pass.output {
                let size = &execution.targets[target];
                (&gpu.targets[target].0, [size.width as f32, size.height as f32, self.atlas_size[0], self.atlas_size[1], 0.0, 0.0, 0.0, 0.0],
                    RECT { left: 0, top: 0, right: size.width as i32, bottom: size.height as i32 })
            } else { (scene, scene_constants, update) };
            upload(&self.context, &self.constants, &constants)?;
            let mut inputs = [None, None, None, None];
            for (slot, target) in inputs.iter_mut().zip(&pass.inputs) { *slot = Some(gpu.targets[*target].1.clone()); }
            let count = pass.draw.count(&values)?;
            // SAFETY: the validated DAG only reads preceding targets; views share the device and draw counts fit the instance buffer.
            unsafe {
                self.context.VSSetShaderResources(1, Some(&[None, None, None, None]));
                self.context.PSSetShaderResources(1, Some(&[None, None, None, None]));
                self.context.OMSetRenderTargets(Some(&[Some(view.clone())]), None);
                if pass.output.is_some() { self.context.ClearRenderTargetView(view, &[0.0; 4]); }
                self.context.VSSetShaderResources(1, Some(&inputs));
                self.context.PSSetShaderResources(1, Some(&inputs));
                self.context.RSSetScissorRects(Some(&[clip]));
                self.context.RSSetViewports(Some(&[D3D11_VIEWPORT { Width: constants[0], Height: constants[1], MaxDepth: 1.0, ..Default::default() }]));
                self.context.OMSetBlendState(blend, None, u32::MAX);
                self.context.IASetPrimitiveTopology(match pass.draw.topology {
                    rdi_core::DrawTopology::Triangles => D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
                    rdi_core::DrawTopology::TriangleStrip => D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
                    rdi_core::DrawTopology::Lines => D3D_PRIMITIVE_TOPOLOGY_LINELIST,
                });
                self.context.IASetInputLayout(&program.layout);
                self.context.VSSetShader(&program.vertex, None);
                self.context.PSSetShader(&program.pixel, None);
                self.context.DrawInstanced(count, 1, 0, index as u32);
            }
        }
        // SAFETY: restore shared scene state and unbind intermediate inputs before another icon reuses those textures.
        unsafe {
            self.context.VSSetShaderResources(1, Some(&[None, None, None, None]));
            self.context.PSSetShaderResources(1, Some(&[None, None, None, None]));
            self.context.OMSetBlendState(&self.blend, None, u32::MAX);
            self.context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn render(
        &mut self,
        surface: &IDXGISurface,
        offset: [f32; 2],
        origin: Point,
        frame: &[IconFrame],
        clean: bool,
    ) -> Result<(), DesktopError> {
        let target: ID3D11Texture2D = surface.cast().map_err(gpu_error)?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: the standalone test texture owns its descriptor.
        unsafe { target.GetDesc(&mut desc) };
        self.render_region(
            surface,
            offset,
            RECT {
                left: 0,
                top: 0,
                right: desc.Width as i32,
                bottom: desc.Height as i32,
            },
            origin,
            frame,
            clean,
        )
    }
}

fn instance_element() -> D3D11_INPUT_ELEMENT_DESC {
    D3D11_INPUT_ELEMENT_DESC {
        Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
        InputSlot: 0,
        InputSlotClass: D3D11_INPUT_PER_INSTANCE_DATA,
        InstanceDataStepRate: 1,
        ..Default::default()
    }
}

fn gpu_program(device: &ID3D11Device, program: &ShaderProgram) -> Result<GpuProgram, DesktopError> {
    let mut vertex = None;
    let mut pixel = None;
    let mut layout = None;
    let count: u32 = match program.pipeline {
        ShaderPipeline::Sprite => 4,
        ShaderPipeline::Particles => 6,
        ShaderPipeline::Procedural => 8,
    };
    let elements: Vec<_> = (0..count)
        .map(|index| D3D11_INPUT_ELEMENT_DESC {
            SemanticName: if index == 0 {
                s!("POSITION")
            } else {
                s!("TEXCOORD")
            },
            SemanticIndex: index.saturating_sub(1),
            AlignedByteOffset: index * 16,
            ..instance_element()
        })
        .collect();
    // SAFETY: owned stage bytecode and POD layout descriptors outlive each call; all objects share this device.
    unsafe {
        device
            .CreateVertexShader(&program.vertex_bytecode, None, Some(&mut vertex))
            .map_err(gpu_error)?;
        device
            .CreatePixelShader(&program.pixel_bytecode, None, Some(&mut pixel))
            .map_err(gpu_error)?;
        device
            .CreateInputLayout(&elements, &program.vertex_bytecode, Some(&mut layout))
            .map_err(gpu_error)?;
    }
    match (vertex, pixel, layout) {
        (Some(vertex), Some(pixel), Some(layout)) => Ok(GpuProgram {
            execution: program.execution.as_ref().map(|execution| prepare_execution(device, execution)).transpose()?,
            vertex,
            pixel,
            layout,
        }),
        _ => Err(DesktopError::InvalidEffect(
            "missing shader pipeline resource".into(),
        )),
    }
}

fn prepare_execution(device: &ID3D11Device, execution: &rdi_core::EffectExecution) -> Result<GpuExecution, DesktopError> {
    let mut targets = Vec::new();
    for target in &execution.targets {
        let texture = texture(device, target.width, target.height)?;
        let mut view = None;
        let mut resource = None;
        // SAFETY: the texture has both required bind flags; output slots are valid and owned views retain it.
        unsafe {
            device.CreateRenderTargetView(&texture, None, Some(&mut view)).map_err(gpu_error)?;
            device.CreateShaderResourceView(&texture, None, Some(&mut resource)).map_err(gpu_error)?;
        }
        targets.push((view.unwrap(), resource.unwrap()));
    }
    let mut passes = Vec::new();
    for pass in &execution.passes {
        let program = gpu_program(device, &ShaderProgram { execution: None, pipeline: ShaderPipeline::Procedural,
            vertex_bytecode: pass.vertex_bytecode.clone(), pixel_bytecode: pass.pixel_bytecode.clone() })?;
        let mut descriptor = D3D11_BLEND_DESC::default();
        let destination = match pass.blend { rdi_core::PassBlend::Over => D3D11_BLEND_INV_SRC_ALPHA,
            rdi_core::PassBlend::Add => D3D11_BLEND_ONE, rdi_core::PassBlend::Replace => D3D11_BLEND_ZERO };
        descriptor.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(), SrcBlend: D3D11_BLEND_ONE, DestBlend: destination, BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE, DestBlendAlpha: destination, BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let mut blend = None;
        // SAFETY: descriptor uses supported premultiplied blend factors and the output slot is valid.
        unsafe { device.CreateBlendState(&descriptor, Some(&mut blend)).map_err(gpu_error)?; }
        passes.push((program, blend.unwrap()));
    }
    Ok(GpuExecution { targets, passes })
}

fn buffer(
    device: &ID3D11Device,
    bytes: u32,
    binding: D3D11_BIND_FLAG,
) -> Result<ID3D11Buffer, DesktopError> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: bytes,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: binding.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        ..Default::default()
    };
    let mut result = None;
    // SAFETY: nonzero buffer size and valid descriptor; result slot outlives the call.
    unsafe { device.CreateBuffer(&desc, None, Some(&mut result)) }.map_err(gpu_error)?;
    result.ok_or_else(|| DesktopError::InvalidEffect("missing buffer".into()))
}

fn upload<T: Copy>(
    context: &ID3D11DeviceContext,
    buffer: &ID3D11Buffer,
    values: &[T],
) -> Result<(), DesktopError> {
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: callers provide initialized POD float data fitting the allocated buffer; mapped memory is writable until Unmap.
    unsafe {
        context
            .Map(buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
            .map_err(gpu_error)?;
        std::ptr::copy_nonoverlapping(
            values.as_ptr().cast::<u8>(),
            mapped.pData.cast::<u8>(),
            size_of_val(values),
        );
        context.Unmap(buffer, 0);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;

    pub(crate) fn device() -> ID3D11Device {
        let mut result = None;
        // SAFETY: valid output slot and hardware device flags; no window or Shell state is touched.
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut result),
                None,
                None,
            )
            .unwrap();
        }
        result.unwrap()
    }

    pub(crate) fn read_pixels(device: &ID3D11Device, target: &ID3D11Texture2D) -> Vec<u8> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        let mut staging = None;
        // SAFETY: matching staging texture dimensions; Map is synchronized with CopyResource and rows are copied before Unmap.
        unsafe {
            target.GetDesc(&mut desc);
            desc.BindFlags = 0;
            desc.Usage = D3D11_USAGE_STAGING;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            device
                .CreateTexture2D(&desc, None, Some(&mut staging))
                .unwrap();
            let staging = staging.unwrap();
            let context = device.GetImmediateContext().unwrap();
            context.CopyResource(&staging, target);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            let mut pixels = Vec::with_capacity((desc.Width * desc.Height * 4) as usize);
            for row in 0..desc.Height {
                pixels.extend_from_slice(std::slice::from_raw_parts(
                    mapped
                        .pData
                        .cast::<u8>()
                        .add((row * mapped.RowPitch) as usize),
                    (desc.Width * 4) as usize,
                ));
            }
            context.Unmap(&staging, 0);
            pixels
        }
    }

    #[test]
    #[ignore = "requires a Windows hardware D3D11 device"]
    fn gpu_update_region_preserves_surroundings() {
        let device = device();
        let atlas = texture(&device, 32, 32).unwrap();
        let target = texture(&device, 96, 80).unwrap();
        let surface = target.cast().unwrap();
        let pixels = [40u8, 120, 220, 255].repeat(32 * 32);
        let sentinel = [19u8, 37, 61, 255].repeat(96 * 80);
        // SAFETY: source buffers cover both complete BGRA textures with matching row strides.
        unsafe {
            let context = device.GetImmediateContext().unwrap();
            context.UpdateSubresource(&atlas, 0, None, pixels.as_ptr().cast(), 128, 0);
        }
        let update = RECT {
            left: 13,
            top: 17,
            right: 61,
            bottom: 57,
        };
        let frame = [IconFrame {
            id: IconId::from("bounded"),
            position: Point::new(-100, -200),
            progress: 0.5,
            elapsed_seconds: 1.0,
        }];
        for pipeline in [ShaderPipeline::Sprite, ShaderPipeline::Particles, ShaderPipeline::Procedural] {
            let program = shader::compile(shader::ShaderSource {
                execution: None,
                pipeline,
                vertex: Some(
                    "VertexOutput vertex(Instance instance, uint vertex_id : SV_VertexID) {
                    VertexOutput output = default_vertex(instance, vertex_id);
                    output.position = float4((output.uv * 2.0 - 1.0) * 100.0, 0, 1);
                    return output;
                }",
                ),
                pixel: Some(
                    "float4 pixel(VertexOutput input) : SV_Target { return float4(0, 1, 0, 1); }",
                ),
            })
            .unwrap();
            let sprite = Sprite {
                id: frame[0].id.clone(),
                origin: frame[0].position,
                destination: frame[0].position,
                region: [0.0, 0.0, 32.0, 32.0],
                anchor: [0.0; 2],
                geometry: [16.0; 4],
                effect: Some(Effect {
                    params: program.default_params(),
                    shader: program,
                    padding_px: 0,
                    envelope: rdi_core::Curve::keyframes(
                        vec![
                            rdi_core::Keyframe::new(0.0, 1.0),
                            rdi_core::Keyframe::new(1.0, 1.0),
                        ],
                        rdi_core::KeyframeInterp::Linear,
                    )
                    .unwrap(),
                    seed: 0.0,
                }),
            };
            let mut renderer = SpriteRenderer::new(&device, &atlas, vec![sprite]).unwrap();
            for clean in [false, true] {
                // SAFETY: sentinel covers the whole 96x80 texture and stays alive through upload.
                unsafe {
                    renderer.context.UpdateSubresource(
                        &target,
                        0,
                        None,
                        sentinel.as_ptr().cast(),
                        384,
                        0,
                    );
                }
                renderer
                    .render_region(
                        &surface,
                        [13.0, 17.0],
                        update,
                        Point::new(-100, -200),
                        &frame,
                        clean,
                    )
                    .unwrap();
                let actual = read_pixels(&device, &target);
                let mut colored = 0;
                let mut cleared = 0;
                for (index, pixel) in actual.chunks_exact(4).enumerate() {
                    let column = (index % 96) as i32;
                    let row = (index / 96) as i32;
                    if (update.left..update.right).contains(&column)
                        && (update.top..update.bottom).contains(&row)
                    {
                        colored += usize::from(pixel[3] > 0);
                        cleared += usize::from(pixel == [0; 4]);
                        if clean {
                            let expected = if column < 45 && row < 49 {
                                [40, 120, 220, 255]
                            } else {
                                [0; 4]
                            };
                            assert_eq!(pixel, expected);
                        } else {
                            assert_eq!(pixel, [0, 255, 0, 255]);
                        }
                    } else {
                        assert_eq!(
                            pixel,
                            &sentinel[index * 4..index * 4 + 4],
                            "outside {pipeline:?}, clean={clean}"
                        );
                    }
                }
                if clean {
                    assert!(colored > 0 && cleared > 0);
                }
            }
            let before = read_pixels(&device, &target);
            for invalid in [
                RECT { left: -1, ..update },
                RECT {
                    right: 97,
                    ..update
                },
                RECT {
                    bottom: 81,
                    ..update
                },
                RECT {
                    right: 13,
                    ..update
                },
                RECT { top: -1, ..update },
                RECT {
                    bottom: 16,
                    ..update
                },
            ] {
                assert!(
                    renderer
                        .render_region(
                            &surface,
                            [13.0, 17.0],
                            invalid,
                            Point::new(-100, -200),
                            &frame,
                            false
                        )
                        .is_err()
                );
                assert!(read_pixels(&device, &target) == before);
            }
        }
        let mut empty = SpriteRenderer::new(&device, &atlas, Vec::new()).unwrap();
        // SAFETY: sentinel covers the full target subresource with the matching stride.
        unsafe {
            empty
                .context
                .UpdateSubresource(&target, 0, None, sentinel.as_ptr().cast(), 384, 0);
        }
        empty
            .render_region(&surface, [13.0, 17.0], update, Point::new(0, 0), &[], false)
            .unwrap();
        let actual = read_pixels(&device, &target);
        for (index, pixel) in actual.chunks_exact(4).enumerate() {
            let inside = (13..61).contains(&(index % 96)) && (17..57).contains(&(index / 96));
            assert_eq!(
                pixel,
                if inside {
                    &[0; 4]
                } else {
                    &sentinel[index * 4..index * 4 + 4]
                }
            );
        }
    }

    #[test]
    #[ignore = "requires a Windows hardware D3D11 device"]
    fn gpu_identity_glitch_and_endpoints() {
        let device = device();
        let atlas = texture(&device, 32, 32).unwrap();
        let mut pixels = vec![0u8; 32 * 32 * 4];
        for row in 4..28 {
            for column in 4..28 {
                let offset = (row * 32 + column) * 4;
                pixels[offset..offset + 4].copy_from_slice(&[40, 120, 220, 255]);
            }
        }
        // SAFETY: tightly packed source pixels cover the complete 32x32 BGRA subresource.
        unsafe {
            device.GetImmediateContext().unwrap().UpdateSubresource(
                &atlas,
                0,
                None,
                pixels.as_ptr().cast(),
                128,
                0,
            )
        };
        let effect = Effect {
            shader: shader::compile(shader::BuiltinShader::Glitch).unwrap(),
            params: [6.0, 3.0, 3.0, 20.0],
            padding_px: 8,
            envelope: rdi_core::Curve::linear(),
            seed: 1.0,
        };
        let sprite = Sprite {
            id: IconId::from("test"),
            origin: Point::new(0, 0),
            destination: Point::new(0, 0),
            region: [0.0, 0.0, 32.0, 32.0],
            anchor: [0.0; 2],
            geometry: [16.0, 16.0, 24.0, 24.0],
            effect: Some(effect),
        };
        let mut renderer = SpriteRenderer::new(&device, &atlas, vec![sprite]).unwrap();
        let target = texture(&device, 32, 32).unwrap();
        let surface: IDXGISurface = target.cast().unwrap();
        let mut frame = vec![IconFrame {
            id: IconId::from("test"),
            position: Point::new(0, 0),
            progress: 0.0,
            elapsed_seconds: 0.0,
        }];
        renderer
            .render(&surface, [0.0; 2], Point::new(0, 0), &frame, false)
            .unwrap();
        assert_eq!(read_pixels(&device, &target), pixels);
        frame[0].progress = 0.8;
        frame[0].elapsed_seconds = 0.15;
        renderer
            .render(&surface, [0.0; 2], Point::new(0, 0), &frame, false)
            .unwrap();
        let changed = read_pixels(&device, &target);
        assert_ne!(changed, pixels);
        assert!(
            changed
                .chunks_exact(4)
                .all(|pixel| pixel[..3].iter().all(|channel| *channel <= pixel[3]))
        );
        frame[0].progress = 1.0;
        renderer
            .render(&surface, [0.0; 2], Point::new(0, 0), &frame, false)
            .unwrap();
        assert_eq!(read_pixels(&device, &target), pixels);
        frame[0].progress = 0.8;
        frame[0].elapsed_seconds = 0.15;
        renderer
            .render(&surface, [0.0; 2], Point::new(0, 0), &frame, false)
            .unwrap();
        assert_eq!(read_pixels(&device, &target), changed);
        frame[0].progress = 0.0;
        frame[0].elapsed_seconds = 0.0;
        renderer
            .render(&surface, [0.0; 2], Point::new(0, 0), &frame, false)
            .unwrap();
        assert_eq!(read_pixels(&device, &target), pixels);
    }

    #[test]
    #[ignore = "requires a Windows hardware D3D11 device"]
    fn gpu_particle_vortex_pixels() {
        let device = device();
        let atlas = texture(&device, 32, 32).unwrap();
        let mut pixels = vec![0u8; 32 * 32 * 4];
        for row in 4..28 {
            for column in 4..28 {
                let offset = (row * 32 + column) * 4;
                pixels[offset..offset + 4].copy_from_slice(&[40, 120, 220, 255]);
            }
        }
        // SAFETY: tightly packed BGRA data covers the entire atlas subresource.
        unsafe {
            device.GetImmediateContext().unwrap().UpdateSubresource(
                &atlas,
                0,
                None,
                pixels.as_ptr().cast(),
                128,
                0,
            );
        }
        let program = shader::compile(shader::BuiltinShader::ParticleVortex).unwrap();
        let effect = Effect {
            params: program.default_params(),
            shader: program,
            padding_px: 0,
            envelope: rdi_core::Curve::keyframes(
                vec![
                    rdi_core::Keyframe::new(0.0, 1.0),
                    rdi_core::Keyframe::new(1.0, 1.0),
                ],
                rdi_core::KeyframeInterp::Linear,
            )
            .unwrap(),
            seed: 7.0,
        };
        let sprites = (0..3)
            .map(|index| Sprite {
                id: IconId::from(format!("particle-{index}")),
                origin: Point::new(index * 96, index * 96),
                destination: Point::new(index * 96, index * 96),
                region: [0.0, 0.0, 32.0, 32.0],
                anchor: [0.0; 2],
                geometry: [16.0, 16.0, 24.0, 24.0],
                effect: (index == 1).then(|| effect.clone()),
            })
            .collect();
        let mut renderer = SpriteRenderer::new(&device, &atlas, sprites).unwrap();
        let target = texture(&device, 256, 256).unwrap();
        let surface = target.cast().unwrap();
        let mut frame = [IconFrame {
            id: IconId::from("particle-1"),
            position: Point::new(96, 96),
            progress: 0.0,
            elapsed_seconds: 0.0,
        }];
        renderer
            .render(&surface, [0.0; 2], Point::new(0, 0), &frame, false)
            .unwrap();
        let baseline = read_pixels(&device, &target);
        let mut middle = Vec::new();
        for progress in [0.01, 0.1, 0.2, 0.5, 0.7, 0.85, 0.99] {
            frame[0].progress = progress;
            frame[0].elapsed_seconds = progress * 4.0;
            renderer
                .render(&surface, [0.0; 2], Point::new(0, 0), &frame, false)
                .unwrap();
            let actual = read_pixels(&device, &target);
            assert_ne!(actual, baseline, "no visible change at {progress}");
            assert!(
                actual
                    .chunks_exact(4)
                    .all(|pixel| pixel[..3].iter().all(|value| *value <= pixel[3]))
            );
            let mut cloud_pixels = 0;
            for row in 0..256 {
                for column in 0..256 {
                    let offset = (row * 256 + column) * 4;
                    if (64..160).contains(&row) && (64..160).contains(&column) {
                        cloud_pixels += usize::from(actual[offset + 3] > 0);
                    } else {
                        assert_eq!(
                            &actual[offset..offset + 4],
                            &baseline[offset..offset + 4],
                            "cloud escaped bounds or changed static artwork"
                        );
                    }
                }
            }
            assert!(cloud_pixels > 30, "cloud disappeared at {progress}");
            if progress == 0.5 {
                assert!(actual.chunks_exact(4).enumerate().any(|(index, pixel)| {
                    let column = index % 256;
                    let row = index / 256;
                    (64..160).contains(&column)
                        && (64..160).contains(&row)
                        && (!(96..128).contains(&column) || !(96..128).contains(&row))
                        && pixel[3] > 0
                }));
                middle = actual;
            }
        }
        for progress in [1.0, 0.0] {
            frame[0].progress = progress;
            renderer
                .render(&surface, [0.0; 2], Point::new(0, 0), &frame, false)
                .unwrap();
            assert_eq!(read_pixels(&device, &target), baseline);
        }
        frame[0].progress = 0.5;
        frame[0].elapsed_seconds = 2.0;
        renderer
            .render(&surface, [0.0; 2], Point::new(0, 0), &frame, false)
            .unwrap();
        assert_eq!(read_pixels(&device, &target), middle);
        renderer
            .render(&surface, [0.0; 2], Point::new(0, 0), &frame, true)
            .unwrap();
        assert_eq!(read_pixels(&device, &target), baseline);
        frame[0].position = Point::new(119, 119);
        renderer
            .render(&surface, [3.0, 5.0], Point::new(10, 20), &frame, false)
            .unwrap();
        let moved = read_pixels(&device, &target);
        for row in 64..160 {
            for column in 64..160 {
                let before = (row * 256 + column) * 4;
                let after = ((row + 8) * 256 + column + 16) * 4;
                assert_eq!(&middle[before..before + 4], &moved[after..after + 4]);
            }
        }
    }

    #[test]
    #[ignore = "hardware throughput benchmark; not displayed FPS"]
    fn gpu_throughput_256_icons_3840x2400() {
        gpu_throughput(shader::compile(shader::BuiltinShader::Glitch).unwrap());
    }

    #[test]
    #[ignore = "requires a Windows hardware D3D11 device"]
    fn gpu_dust_transfer_directional_reassembly() {
        let device = device();
        let atlas = texture(&device, 128, 128).unwrap();
        let mut pixels = vec![0u8; 128 * 128 * 4];
        for row in 0..128 {
            for column in 0..128 {
                let color = if row < 96 {
                    [40, 120, 220, 255]
                } else if (104..120).contains(&row) && (8..120).contains(&column) {
                    [220, 200, 80, 255]
                } else {
                    [0; 4]
                };
                let offset = (row * 128 + column) * 4;
                pixels[offset..offset + 4].copy_from_slice(&color);
            }
        }
        // SAFETY: tightly packed BGRA data covers the entire 128x128 atlas subresource.
        unsafe {
            device.GetImmediateContext().unwrap().UpdateSubresource(
                &atlas,
                0,
                None,
                pixels.as_ptr().cast(),
                512,
                0,
            );
        }
        let program = shader::compile(shader::BuiltinShader::DustTransfer).unwrap();
        let effect = Effect {
            params: [2.0, 1.5, 2.0, 0.65],
            shader: program,
            padding_px: 0,
            envelope: rdi_core::Curve::keyframes(
                vec![
                    rdi_core::Keyframe::new(0.0, 1.0),
                    rdi_core::Keyframe::new(1.0, 1.0),
                ],
                rdi_core::KeyframeInterp::Linear,
            )
            .unwrap(),
            seed: 7.0,
        };
        let mut renderer = SpriteRenderer::new(
            &device,
            &atlas,
            vec![
                Sprite {
                    id: IconId::from("dust"),
                    origin: Point::new(32, 64),
                    destination: Point::new(352, 64),
                    region: [0.0, 0.0, 128.0, 128.0],
                    anchor: [0.0; 2],
                    geometry: [64.0, 48.0, 128.0, 96.0],
                    effect: Some(effect),
                },
                Sprite {
                    id: IconId::from("static"),
                    origin: Point::new(0, 0),
                    destination: Point::new(0, 0),
                    region: [0.0, 0.0, 16.0, 16.0],
                    anchor: [0.0; 2],
                    geometry: [8.0, 8.0, 16.0, 16.0],
                    effect: None,
                },
            ],
        )
        .unwrap();
        let target = texture(&device, 512, 256).unwrap();
        let surface = target.cast().unwrap();
        let mut frame = [IconFrame {
            id: IconId::from("dust"),
            position: Point::new(32, 64),
            progress: 0.0,
            elapsed_seconds: 0.0,
        }];
        let mut snapshots = Vec::new();
        for progress in [0.0, 0.15, 0.4, 0.5, 0.6, 0.85, 1.0] {
            frame[0].position = Point::new(32 + (320.0 * progress) as i32, 64);
            frame[0].progress = progress;
            frame[0].elapsed_seconds = progress * 4.0;
            renderer
                .render(&surface, [0.0; 2], Point::new(0, 0), &frame, false)
                .unwrap();
            let actual = read_pixels(&device, &target);
            assert!(
                actual
                    .chunks_exact(4)
                    .all(|pixel| pixel[..3].iter().all(|value| *value <= pixel[3]))
            );
            for row in 0..16 {
                for column in 0..16 {
                    let offset = (row * 512 + column) * 4;
                    assert_eq!(&actual[offset..offset + 4], &[40, 120, 220, 255]);
                }
            }
            let visible = actual.chunks_exact(4).filter(|pixel| pixel[3] > 0).count();
            assert!(visible > 500, "dust vanished at {progress}: {visible}");
            if progress == 0.0 || progress == 1.0 {
                renderer
                    .render(&surface, [0.0; 2], Point::new(0, 0), &frame, true)
                    .unwrap();
                assert_eq!(actual, read_pixels(&device, &target));
            }
            snapshots.push(actual);
        }
        let opaque_halves = |image: &[u8], left: usize| {
            let mut halves = [0usize; 4];
            for row in 64..160 {
                for column in left..left + 128 {
                    if image[(row * 512 + column) * 4 + 3] == 255 {
                        halves[usize::from(column >= left + 64)] += 1;
                        halves[2 + usize::from(row >= 112)] += 1;
                    }
                }
            }
            halves[0]
                .abs_diff(halves[1])
                .max(halves[2].abs_diff(halves[3]))
        };
        assert!(
            opaque_halves(&snapshots[1], 32) > 500,
            "breakup was not side-first"
        );
        assert!(
            opaque_halves(&snapshots[5], 352) > 500,
            "assembly was not side-first"
        );
        let centroid = |image: &[u8]| {
            let mut weight = 0.0;
            let mut moment = 0.0;
            for (index, pixel) in image.chunks_exact(4).enumerate().skip(32 * 512) {
                weight += f64::from(pixel[3]);
                moment += (index % 512) as f64 * f64::from(pixel[3]);
            }
            moment / weight
        };
        assert!(centroid(&snapshots[2]) < centroid(&snapshots[3]));
        assert!(centroid(&snapshots[3]) < centroid(&snapshots[4]));
        let middle = &snapshots[3];
        assert!(
            middle
                .chunks_exact(4)
                .enumerate()
                .any(|(index, pixel)| (160..352).contains(&(index % 512)) && pixel[3] > 0)
        );
        let opaque_dust = middle
            .chunks_exact(4)
            .filter(|pixel| pixel[3] == 255)
            .count();
        let opaque_artwork = snapshots[0]
            .chunks_exact(4)
            .filter(|pixel| pixel[3] == 255)
            .count();
        assert!(
            opaque_dust < opaque_artwork / 2,
            "dust retained intact coverage: {opaque_dust} versus {opaque_artwork}"
        );
        assert!(
            middle
                .chunks_exact(4)
                .any(|pixel| pixel[3] > 20 && pixel[0] > pixel[2]),
            "label-colored dust disappeared"
        );
        frame[0].progress = 0.5;
        frame[0].elapsed_seconds = 2.0;
        frame[0].position = Point::new(192, 64);
        renderer
            .render(&surface, [0.0; 2], Point::new(0, 0), &frame, false)
            .unwrap();
        assert!(
            &read_pixels(&device, &target) == middle,
            "rewind changed dust trajectories"
        );
        frame[0].position = Point::new(250, 100);
        renderer
            .render(&surface, [0.0; 2], Point::new(0, 0), &frame, false)
            .unwrap();
        assert!(
            &read_pixels(&device, &target) == middle,
            "full-strength dust followed current position instead of endpoints"
        );
        renderer.sprites[0].effect.as_mut().unwrap().seed = 19.0;
        renderer
            .render(&surface, [0.0; 2], Point::new(0, 0), &frame, false)
            .unwrap();
        assert_ne!(&read_pixels(&device, &target), middle, "seed had no effect");
        renderer
            .render(&surface, [0.0; 2], Point::new(0, 0), &frame, true)
            .unwrap();
        let clean = read_pixels(&device, &target);
        renderer.sprites[0].effect.as_mut().unwrap().envelope = rdi_core::Curve::keyframes(
            vec![
                rdi_core::Keyframe::new(0.0, 0.0),
                rdi_core::Keyframe::new(1.0, 0.0),
            ],
            rdi_core::KeyframeInterp::Linear,
        )
        .unwrap();
        renderer
            .render(&surface, [0.0; 2], Point::new(0, 0), &frame, false)
            .unwrap();
        assert_eq!(read_pixels(&device, &target), clean);
    }

    #[test]
    #[ignore = "requires a Windows hardware D3D11 device"]
    fn gpu_procedural_multipass_resources_and_rewind() {
        use rdi_core::{DrawSpec, DrawTopology, EffectParameter, EffectTarget, ParameterKind, PassBlend};
        let device = device();
        let atlas = texture(&device, 32, 32).unwrap();
        let pixels = [40u8, 120, 220, 255].repeat(32 * 32);
        // SAFETY: initialized BGRA pixels cover the complete atlas on its owning device.
        unsafe { device.GetImmediateContext().unwrap().UpdateSubresource(&atlas, 0, None, pixels.as_ptr().cast(), 128, 0); }
        let execution = shader::ExecutionSource {
            parameters: (0..5).map(|index| EffectParameter { name: format!("value{index}"), kind: ParameterKind::Float,
                default: if index == 4 { 0.5 } else { 0.0 }, min: 0.0, max: 1.0 }).collect(),
            targets: vec![EffectTarget { width: 16, height: 16 }],
            passes: vec![
                shader::PassSource {
                    vertex: Some("VertexOutput vertex(Instance instance, uint vertex_id : SV_VertexID) {
                        VertexOutput output = default_vertex(instance, 0);
                        float2 uv = float2(vertex_id % 2, vertex_id / 2);
                        output.position = float4(uv.x * 2 - 1, 1 - uv.y * 2, 0, 1);
                        output.uv = uv; return output;
                    }".into()),
                    pixel: Some("float4 pixel(VertexOutput input) : SV_Target {
                        return float4(input.timing.y * 0.5, parameter(4), 0, 0.5);
                    }".into()),
                    draw: DrawSpec { vertices: 4, topology: DrawTopology::TriangleStrip, ..Default::default() },
                    output: Some(0), blend: PassBlend::Over, ..Default::default()
                },
                shader::PassSource {
                    pixel: Some("float4 pixel(VertexOutput input) : SV_Target {
                        return pass_inputs[0].SampleLevel(artwork_sampler, input.uv, 0);
                    }".into()), ..Default::default()
                }
            ], ..Default::default()
        };
        let mut execution = execution;
        execution.passes[1].inputs = vec![0];
        let program = shader::compile(shader::ShaderSource { execution: Some(std::sync::Arc::new(execution)),
            pipeline: ShaderPipeline::Procedural, vertex: None, pixel: None }).unwrap();
        let effect = Effect { params: program.default_params(), shader: program, padding_px: 0, seed: 0.0,
            envelope: rdi_core::Curve::keyframes(vec![rdi_core::Keyframe::new(0.0, 1.0), rdi_core::Keyframe::new(1.0, 1.0)], rdi_core::KeyframeInterp::Linear).unwrap() };
        let sprites = (0..2).map(|index| Sprite {
            id: IconId::from(format!("graph{index}")), origin: Point::new(16 + index * 40, 16), destination: Point::new(16 + index * 40, 16),
            region: [0.0, 0.0, 32.0, 32.0], anchor: [0.0; 2], geometry: [16.0, 16.0, 32.0, 32.0], effect: Some(effect.clone()),
        }).collect();
        let mut renderer = SpriteRenderer::new(&device, &atlas, sprites).unwrap();
        let target = texture(&device, 128, 80).unwrap();
        let surface = target.cast().unwrap();
        let mut frames: Vec<_> = (0..2).map(|index| IconFrame { id: IconId::from(format!("graph{index}")), position: Point::new(16 + index * 40, 16),
            progress: 0.5, elapsed_seconds: 1.0 }).collect();
        renderer.render(&surface, [3.0, 5.0], Point::new(0, 0), &frames, false).unwrap();
        let middle = read_pixels(&device, &target);
        for column in [20, 60] {
            let pixel = &middle[(25 * 128 + column) * 4..(25 * 128 + column) * 4 + 4];
            assert!(pixel.iter().zip([0u8, 128, 64, 128]).all(|(actual, expected)| actual.abs_diff(expected) <= 1), "unexpected BGRA: {pixel:?}");
        }
        assert_eq!(&middle[(16 * 128 + 16) * 4..(16 * 128 + 16) * 4 + 4], &[0; 4]);
        for frame in &mut frames { frame.progress = 0.8; }
        renderer.render(&surface, [3.0, 5.0], Point::new(0, 0), &frames, false).unwrap();
        assert!(read_pixels(&device, &target) != middle);
        for frame in &mut frames { frame.progress = 0.5; }
        renderer.render(&surface, [3.0, 5.0], Point::new(0, 0), &frames, false).unwrap();
        assert!(read_pixels(&device, &target) == middle);
        renderer.render(&surface, [3.0, 5.0], Point::new(0, 0), &frames, true).unwrap();
        let clean = read_pixels(&device, &target);
        assert_eq!(&clean[(25 * 128 + 20) * 4..(25 * 128 + 20) * 4 + 4], &[40, 120, 220, 255]);
        for frame in &mut frames { frame.progress = 0.0; }
        renderer.render(&surface, [3.0, 5.0], Point::new(0, 0), &frames, false).unwrap();
        assert!(read_pixels(&device, &target) == clean);
        let sentinel = [7u8, 11, 13, 255].repeat(128 * 80);
        // SAFETY: initialized pixels cover the complete target and remain alive through UpdateSubresource.
        unsafe { device.GetImmediateContext().unwrap().UpdateSubresource(&target, 0, None, sentinel.as_ptr().cast(), 128 * 4, 0); }
        for frame in &mut frames { frame.progress = 0.5; }
        let clip = RECT { left: 23, top: 25, right: 43, bottom: 41 };
        renderer.render_region(&surface, [3.0, 5.0], clip, Point::new(0, 0), &frames, false).unwrap();
        let clipped = read_pixels(&device, &target);
        for (index, pixel) in clipped.chunks_exact(4).enumerate() {
            let inside = (23..43).contains(&(index % 128)) && (25..41).contains(&(index / 128));
            let expected = if inside { &middle[index * 4..index * 4 + 4] } else { &sentinel[index * 4..index * 4 + 4] };
            assert_eq!(pixel, expected, "partial update at pixel {index}");
        }
    }

    #[test]
    #[ignore = "requires a Windows hardware D3D11 device"]
    fn gpu_optional_stages_and_sprite_geometry() {
        let device = device();
        let atlas = texture(&device, 32, 32).unwrap();
        let pixels = [40u8, 120, 220, 255].repeat(32 * 32);
        // SAFETY: tightly packed BGRA data covers the entire atlas subresource.
        unsafe {
            device.GetImmediateContext().unwrap().UpdateSubresource(
                &atlas,
                0,
                None,
                pixels.as_ptr().cast(),
                128,
                0,
            );
        }
        let vertex = "VertexOutput vertex(Instance instance, uint vertex_id : SV_VertexID) {
            VertexOutput output = default_vertex(instance, vertex_id);
            output.position.x += 32.0 / dimensions.x;
            return output;
        }";
        let pixel = "float4 pixel(VertexOutput input) : SV_Target { return float4(0, 1, 0, 1); }";
        let target = texture(&device, 96, 64).unwrap();
        let surface = target.cast().unwrap();
        for pipeline in [ShaderPipeline::Sprite, ShaderPipeline::Particles] {
            for vertex in [None, Some(vertex)] {
                for pixel in [None, Some(pixel)] {
                    let program = shader::compile(shader::ShaderSource {
                        execution: None,
                        pipeline,
                        vertex,
                        pixel,
                    })
                    .unwrap();
                    let sprite = Sprite {
                        id: IconId::from("optional"),
                        origin: Point::new(16, 16),
                        destination: Point::new(16, 16),
                        region: [0.0, 0.0, 32.0, 32.0],
                        anchor: [0.0; 2],
                        geometry: [16.0, 16.0, 32.0, 32.0],
                        effect: Some(Effect {
                            params: program.default_params(),
                            shader: program,
                            padding_px: 0,
                            envelope: rdi_core::Curve::keyframes(
                                vec![
                                    rdi_core::Keyframe::new(0.0, 1.0),
                                    rdi_core::Keyframe::new(1.0, 1.0),
                                ],
                                rdi_core::KeyframeInterp::Linear,
                            )
                            .unwrap(),
                            seed: 0.0,
                        }),
                    };
                    let mut renderer = SpriteRenderer::new(&device, &atlas, vec![sprite]).unwrap();
                    for (progress, clean, zero_strength) in [
                        (0.5, false, false),
                        (0.0, false, false),
                        (1.0, false, false),
                        (0.5, true, false),
                        (0.5, false, true),
                    ] {
                        if zero_strength {
                            renderer.sprites[0].effect.as_mut().unwrap().envelope =
                                rdi_core::Curve::keyframes(
                                    vec![
                                        rdi_core::Keyframe::new(0.0, 0.0),
                                        rdi_core::Keyframe::new(1.0, 0.0),
                                    ],
                                    rdi_core::KeyframeInterp::Linear,
                                )
                                .unwrap();
                        }
                        let frame = [IconFrame {
                            id: IconId::from("optional"),
                            position: Point::new(26, 36),
                            progress,
                            elapsed_seconds: progress * 4.0,
                        }];
                        renderer
                            .render(&surface, [3.0, 5.0], Point::new(10, 20), &frame, clean)
                            .unwrap();
                        let active = progress == 0.5 && !clean && !zero_strength;
                        let left = 19 + if active && vertex.is_some() { 16 } else { 0 };
                        let color = if active && pixel.is_some() {
                            [0, 255, 0, 255]
                        } else {
                            [40, 120, 220, 255]
                        };
                        let mut expected = vec![0u8; 96 * 64 * 4];
                        for row in 21..53 {
                            for column in left..left + 32 {
                                let offset = (row * 96 + column) * 4;
                                expected[offset..offset + 4].copy_from_slice(&color);
                            }
                        }
                        assert_eq!(
                            read_pixels(&device, &target),
                            expected,
                            "{pipeline:?}, vertex={}, pixel={}, progress={progress}, clean={clean}, zero={zero_strength}",
                            vertex.is_some(),
                            pixel.is_some()
                        );
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "requires a Windows hardware D3D11 device"]
    fn gpu_custom_particle_vertex_programs_are_distinct() {
        let device = device();
        let atlas = texture(&device, 32, 32).unwrap();
        let pixels = [40u8, 120, 220, 255].repeat(32 * 32);
        // SAFETY: source data covers the complete tightly packed BGRA texture.
        unsafe {
            device.GetImmediateContext().unwrap().UpdateSubresource(
                &atlas,
                0,
                None,
                pixels.as_ptr().cast(),
                128,
                0,
            );
        }
        let programs: Vec<_> = [0, 48]
            .into_iter()
            .map(|shift| {
                let source = shader::BuiltinShader::ParticleVortex.source();
                let vertex = source.vertex.unwrap().replace(
                    "float2 center = lerp(source, orbit, amount);",
                    &format!("float2 center = source + float2({shift}.0, 0.0);"),
                );
                shader::compile(shader::ShaderSource {
                    execution: None,
                    vertex: Some(&vertex),
                    ..source
                })
                .unwrap()
            })
            .collect();
        assert_eq!(programs[0].pixel_bytecode, programs[1].pixel_bytecode);
        assert_ne!(programs[0].vertex_bytecode, programs[1].vertex_bytecode);
        let sprites: Vec<_> = (0..3)
            .map(|index| {
                let program = programs[index % 2].clone();
                Sprite {
                    id: IconId::from(format!("custom-{index}")),
                    origin: if index == 2 {
                        Point::new(0, 0)
                    } else {
                        Point::new(96, 96)
                    },
                    destination: Point::new(96, 96),
                    region: [0.0, 0.0, 32.0, 32.0],
                    anchor: [0.0; 2],
                    geometry: [16.0, 16.0, 24.0, 24.0],
                    effect: Some(Effect {
                        params: program.default_params(),
                        shader: program,
                        padding_px: 0,
                        envelope: rdi_core::Curve::linear(),
                        seed: 0.0,
                    }),
                }
            })
            .collect();
        let frame: Vec<_> = sprites
            .iter()
            .map(|sprite| IconFrame {
                id: sprite.id.clone(),
                position: sprite.origin,
                progress: 0.5,
                elapsed_seconds: 2.0,
            })
            .collect();
        let mut renderer = SpriteRenderer::new(&device, &atlas, sprites).unwrap();
        assert_eq!(renderer.programs.len(), 3);
        let target = texture(&device, 256, 256).unwrap();
        renderer
            .render(
                &target.cast().unwrap(),
                [0.0; 2],
                Point::new(0, 0),
                &frame,
                false,
            )
            .unwrap();
        let actual = read_pixels(&device, &target);
        let mut visible = 0;
        for row in 96..128 {
            for column in 96..128 {
                let first = (row * 256 + column) * 4;
                let second = first + 48 * 4;
                assert_eq!(&actual[first..first + 4], &actual[second..second + 4]);
                visible += usize::from(actual[first + 3] > 0);
            }
        }
        assert!(visible > 100, "custom vertex programs were not used");
    }

    #[test]
    #[ignore = "hardware throughput benchmark; not displayed FPS"]
    fn gpu_particle_throughput_256_icons_3840x2400() {
        gpu_throughput(shader::compile(shader::BuiltinShader::ParticleVortex).unwrap());
    }

    #[test]
    #[ignore = "hardware throughput benchmark; not displayed FPS"]
    fn gpu_silk_flow_throughput_256_icons_3840x2400() {
        gpu_throughput(shader::compile(shader::BuiltinShader::SilkFlow).unwrap());
    }

    fn gpu_throughput(program: ShaderProgram) {
        let device = device();
        let atlas = texture(&device, 192, 256).unwrap();
        let pixels = vec![180u8; 192 * 256 * 4];
        // SAFETY: the source contains the entire tightly packed BGRA texture.
        unsafe {
            device.GetImmediateContext().unwrap().UpdateSubresource(
                &atlas,
                0,
                None,
                pixels.as_ptr().cast(),
                192 * 4,
                0,
            )
        };
        let effect = Effect {
            params: program.default_params(),
            shader: program,
            padding_px: 16,
            envelope: rdi_core::Curve::keyframes(
                vec![
                    rdi_core::Keyframe::new(0.0, 0.0),
                    rdi_core::Keyframe::new(0.15, 1.0),
                    rdi_core::Keyframe::new(0.85, 1.0),
                    rdi_core::Keyframe::new(1.0, 0.0),
                ],
                rdi_core::KeyframeInterp::SmoothStep,
            )
            .unwrap(),
            seed: 3.0,
        };
        let sprites: Vec<_> = (0..256)
            .map(|index| Sprite {
                id: IconId::from(format!("icon-{index}")),
                origin: Point::new((index % 16) * 220, (index / 16) * 140),
                destination: Point::new((index % 16) * 220 + if effect.shader.pipeline == ShaderPipeline::Procedural { 160 } else { 0 }, (index / 16) * 140),
                region: [0.0, 0.0, 192.0, 256.0],
                anchor: [0.0; 2],
                geometry: [96.0, 96.0, 120.0, 120.0],
                effect: Some(effect.clone()),
            })
            .collect();
        let mut frame: Vec<_> = sprites
            .iter()
            .map(|sprite| IconFrame {
                id: sprite.id.clone(),
                position: sprite.origin,
                progress: 0.5,
                elapsed_seconds: 0.0,
            })
            .collect();
        let mut renderer = SpriteRenderer::new(&device, &atlas, sprites).unwrap();
        let target = texture(&device, 3840, 2400).unwrap();
        let surface: IDXGISurface = target.cast().unwrap();
        // SAFETY: the device is live and used on this thread.
        let context = unsafe { device.GetImmediateContext() }.unwrap();
        let mut staging = None;
        let descriptor = D3D11_TEXTURE2D_DESC {
            Width: 1,
            Height: 1,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            ..Default::default()
        };
        // SAFETY: valid 1x1 CPU-readable staging descriptor.
        unsafe {
            device
                .CreateTexture2D(&descriptor, None, Some(&mut staging))
                .unwrap();
        }
        let staging = staging.unwrap();
        for enabled in [false, true] {
            let mut timings = Vec::new();
            for iteration in 0..130 {
                for entry in &mut frame {
                    entry.elapsed_seconds = iteration as f32 / 144.0;
                }
                let start = std::time::Instant::now();
                renderer
                    .render(&surface, [0.0; 2], Point::new(0, 0), &frame, !enabled)
                    .unwrap();
                // SAFETY: copy a valid source pixel; blocking Map waits for preceding GPU work, without a query spin loop.
                unsafe {
                    context.CopySubresourceRegion(
                        &staging,
                        0,
                        0,
                        0,
                        0,
                        &target,
                        0,
                        Some(&D3D11_BOX {
                            left: 0,
                            top: 0,
                            front: 0,
                            right: 1,
                            bottom: 1,
                            back: 1,
                        }),
                    );
                    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                    context
                        .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                        .unwrap();
                    context.Unmap(&staging, 0);
                }
                if iteration >= 10 {
                    timings.push(start.elapsed().as_secs_f64() * 1000.0);
                }
            }
            timings.sort_by(f64::total_cmp);
            let mean = timings.iter().sum::<f64>() / timings.len() as f64;
            println!(
                "256 sprites 3840x2400 {:?} enabled={enabled}: mean={mean:.3}ms p95={:.3}ms throughput={:.1}fps",
                effect.shader.pipeline,
                timings[114],
                1000.0 / mean
            );
        }
    }
}
