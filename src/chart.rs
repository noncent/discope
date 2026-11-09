use crate::disk_scanner::{DiskUsage, Kind, NodeId};
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use std::f32::consts::TAU;

const START: f32 = -0.33;
const MIN_ANGLE: f32 = 0.022;
const WIDTHS: [f32; 12] = [
    0.174, 0.158, 0.143, 0.116, 0.092, 0.075, 0.059, 0.051, 0.043, 0.035, 0.029, 0.025,
];
const PALETTE: [Color32; 12] = [
    Color32::from_rgb(255, 186, 132),
    Color32::from_rgb(208, 235, 131),
    Color32::from_rgb(144, 229, 125),
    Color32::from_rgb(96, 229, 174),
    Color32::from_rgb(94, 222, 218),
    Color32::from_rgb(100, 179, 249),
    Color32::from_rgb(120, 135, 253),
    Color32::from_rgb(160, 119, 248),
    Color32::from_rgb(205, 116, 239),
    Color32::from_rgb(238, 118, 210),
    Color32::from_rgb(247, 116, 162),
    Color32::from_rgb(249, 137, 119),
];

pub const FILE: Color32 = Color32::from_rgb(123, 139, 166);
pub const SMALL: Color32 = Color32::from_rgb(78, 95, 126);
pub const RESTRICTED: Color32 = Color32::from_rgb(185, 148, 246);

pub fn row_color(kind: Kind, rank: usize) -> Color32 {
    match kind {
        Kind::Folder => PALETTE[rank % PALETTE.len()],
        Kind::File => FILE,
        Kind::Unreadable => RESTRICTED,
        Kind::OtherVolume => SMALL,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    Node(NodeId),
    Smaller { parent: NodeId, first: NodeId },
}

#[derive(Clone, Copy)]
struct Sector {
    target: Target,
    angle: [f32; 2],
    radius: [f32; 2],
    color: Color32,
}

struct ChartKey {
    rect: Rect,
    focus: NodeId,
    depth: usize,
    generation: u64,
}

#[derive(Default)]
pub struct ChartCache {
    key: Option<ChartKey>,
    pub center: Pos2,
    pub hub: f32,
    outer: f32,
    mesh: egui::Mesh,
    sectors: Vec<Sector>,
}

impl ChartCache {
    pub fn prepare(
        &mut self,
        rect: Rect,
        usage: &DiskUsage,
        focus: NodeId,
        depth: usize,
        generation: u64,
    ) {
        if self.key.as_ref().is_some_and(|key| {
            key.rect == rect
                && key.focus == focus
                && key.depth == depth
                && key.generation == generation
        }) {
            return;
        }
        self.key = Some(ChartKey {
            rect,
            focus,
            depth,
            generation,
        });
        self.center = rect.center();
        self.outer = (rect.width().min(rect.height()) * 0.47).max(1.0);
        self.hub = self.outer * 0.174;
        self.sectors.clear();

        let mut rings = [0.0; WIDTHS.len() + 1];
        rings[0] = self.hub;
        let width = WIDTHS.iter().sum::<f32>();
        for (index, weight) in WIDTHS.iter().enumerate() {
            rings[index + 1] = rings[index] + (self.outer - self.hub) * weight / width;
        }

        let mut builder = Builder {
            usage,
            rings,
            depth: depth.min(WIDTHS.len()),
            sectors: &mut self.sectors,
        };
        builder.branch(focus, [0.0, TAU], 0, Color32::WHITE);
        let mut mesh = egui::Mesh::default();
        for sector in &self.sectors {
            sector.mesh(&mut mesh, self.center, sector.color);
        }
        self.mesh = mesh;
    }

    pub fn smaller_objects(&self, focus: NodeId) -> Option<NodeId> {
        self.sectors.iter().find_map(|sector| match sector.target {
            Target::Smaller { parent, first } if parent == focus => Some(first),
            _ => None,
        })
    }

    pub fn hit(&self, pos: Pos2) -> Option<Target> {
        let delta = pos - self.center;
        let radius = delta.length();
        let angle = (delta.y.atan2(delta.x) - START).rem_euclid(TAU);
        self.sectors
            .iter()
            .find(|sector| {
                radius >= sector.radius[0]
                    && radius <= sector.radius[1]
                    && angle >= sector.angle[0]
                    && angle <= sector.angle[1]
            })
            .map(|sector| sector.target)
    }

    pub fn draw(&self, painter: &egui::Painter, highlighted: Option<Target>) {
        painter.add(egui::Shape::Mesh(self.mesh.clone()));
        if let Some(highlighted) = highlighted {
            if let Some(sector) = self
                .sectors
                .iter()
                .find(|sector| sector.target == highlighted)
            {
                let mut mesh = egui::Mesh::default();
                sector.mesh(
                    &mut mesh,
                    self.center,
                    mix(sector.color, Color32::WHITE, 0.34),
                );
                painter.add(egui::Shape::mesh(mesh));
            }
        }
    }
}

struct Builder<'a> {
    usage: &'a DiskUsage,
    rings: [f32; WIDTHS.len() + 1],
    depth: usize,
    sectors: &'a mut Vec<Sector>,
}

impl Builder<'_> {
    fn branch(&mut self, parent: NodeId, span: [f32; 2], level: usize, inherited: Color32) {
        let parent_size = self.usage.nodes[parent].size;
        if level >= self.depth || parent_size == 0 {
            return;
        }

        let mut angle = span[0];
        let mut consumed = 0_u64;
        for (rank, id) in self.usage.children(parent).enumerate() {
            let node = &self.usage.nodes[id];
            if node.size == 0 {
                break;
            }
            let end = span[0]
                + (span[1] - span[0])
                    * (consumed.saturating_add(node.size) as f64 / parent_size as f64) as f32;
            if end - angle < MIN_ANGLE {
                if span[1] - angle > 0.003 {
                    self.push(
                        Target::Smaller { parent, first: id },
                        [angle, span[1]],
                        level,
                        SMALL,
                    );
                }
                break;
            }

            let family = if level == 0 {
                row_color(node.kind, rank)
            } else {
                inherited
            };
            let color = match node.kind {
                Kind::File if level != 0 => mix(FILE, family, 0.31),
                Kind::Folder => mix(
                    family,
                    Color32::from_rgb(19, 38, 66),
                    0.045 * (level % 3) as f32,
                ),
                _ => row_color(node.kind, rank),
            };
            self.push(Target::Node(id), [angle, end], level, color);
            if node.kind == Kind::Folder {
                self.branch(id, [angle, end], level + 1, family);
            }
            consumed = consumed.saturating_add(node.size);
            angle = end;
        }
    }

    fn push(&mut self, target: Target, angle: [f32; 2], level: usize, color: Color32) {
        let padding = (0.62 / self.rings[level]).min((angle[1] - angle[0]) * 0.17);
        self.sectors.push(Sector {
            target,
            angle: [angle[0] + padding, angle[1] - padding],
            radius: [self.rings[level] + 0.42, self.rings[level + 1] - 0.42],
            color,
        });
    }
}

impl Sector {
    fn mesh(self, mesh: &mut egui::Mesh, center: Pos2, color: Color32) {
        let sweep = self.angle[1] - self.angle[0];
        // Limit chord error on broad arcs without tessellating each tiny petal 64 times.
        let steps = (sweep * (self.radius[1] / 1.5).sqrt()).ceil().max(1.0) as u32;
        let start = mesh.vertices.len() as u32;
        for index in 0..=steps {
            let radians = self.angle[0] + START + sweep * index as f32 / steps as f32;
            let (sin, cos) = radians.sin_cos();
            let unit = Vec2::new(cos, sin);
            mesh.colored_vertex(center + unit * self.radius[0], color);
            mesh.colored_vertex(center + unit * self.radius[1], color);
            if index < steps {
                let first = start + index * 2;
                mesh.add_triangle(first, first + 1, first + 3);
                mesh.add_triangle(first, first + 3, first + 2);
            }
        }
    }
}

fn mix(a: Color32, b: Color32, fraction: f32) -> Color32 {
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * fraction) as u8;
    Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk_scanner::{scan, tests::Fixture, ScanControl};

    #[test]
    fn a_full_circle_is_clickable_across_the_angle_seam() {
        let fixture = Fixture::new();
        fixture.write("folder/file", 8192);
        let usage = scan(&fixture.0, &ScanControl::default()).unwrap();
        let mut cache = ChartCache::default();
        cache.prepare(
            Rect::from_min_size(Pos2::ZERO, Vec2::splat(500.0)),
            &usage,
            0,
            10,
            0,
        );
        let first = usage.children(0).next().unwrap();
        let radius = cache.hub + 12.0;
        for angle in [START + 0.1, START - 0.1, 2.8, -2.8] {
            let (sin, cos) = angle.sin_cos();
            assert_eq!(
                cache.hit(cache.center + Vec2::new(cos, sin) * radius),
                Some(Target::Node(first))
            );
        }
        assert_eq!(cache.hit(cache.center), None);
    }

    #[test]
    fn small_files_are_grouped_without_losing_their_bytes() {
        let fixture = Fixture::new();
        fixture.write("large", 4 * 1024 * 1024);
        for i in 0..16 {
            fixture.write(&format!("small-{i}"), 512);
        }
        let usage = scan(&fixture.0, &ScanControl::default()).unwrap();
        let mut cache = ChartCache::default();
        cache.prepare(
            Rect::from_min_size(Pos2::ZERO, Vec2::splat(500.0)),
            &usage,
            0,
            10,
            0,
        );
        let range = usage.children(0);
        let group = cache
            .sectors
            .iter()
            .find(|sector| matches!(sector.target, Target::Smaller { .. }))
            .unwrap();
        assert_eq!(
            group.target,
            Target::Smaller {
                parent: 0,
                first: range.start + 1
            }
        );
        assert_eq!(
            usage.nodes[range.start].size
                + usage.nodes[range.start + 1..range.end]
                    .iter()
                    .map(|node| node.size)
                    .sum::<u64>(),
            usage.size()
        );
    }
}
