//! Geometry for stacked graphics markup. The drawing backend supplies glyph metrics.
use super::TextAlign;
use super::text_markup::{Node, Run};

pub struct MeasuredRun<T> {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub glyph: T,
}

pub struct PositionedRun<T> {
    pub origin: [f32; 2],
    pub glyph: T,
}

pub struct Rule {
    pub start: [f32; 2],
    pub end: [f32; 2],
    pub thickness: f32,
}

pub struct Layout<T> {
    pub size: [f32; 2],
    pub glyphs: Vec<PositionedRun<T>>,
    pub rules: Vec<Rule>,
}

pub fn has_boxes(nodes: &[Node]) -> bool {
    nodes.iter().any(|node| !matches!(node, Node::Run(_)))
}

struct Row<T> {
    width: f32,
    ascent: f32,
    descent: f32,
    glyphs: Vec<PositionedRun<T>>,
    rules: Vec<Rule>,
}

impl<T> Default for Row<T> {
    fn default() -> Self {
        Self {
            width: 0.,
            ascent: 0.,
            descent: 0.,
            glyphs: Vec::new(),
            rules: Vec::new(),
        }
    }
}

fn translate([x, y]: &mut [f32; 2], [dx, dy]: [f32; 2]) {
    *x += dx;
    *y += dy;
}

impl<T> Row<T> {
    fn append(&mut self, mut contents: Self, offset: [f32; 2]) {
        for run in &mut contents.glyphs {
            translate(&mut run.origin, offset);
        }
        for rule in &mut contents.rules {
            translate(&mut rule.start, offset);
            translate(&mut rule.end, offset);
        }
        self.glyphs.append(&mut contents.glyphs);
        self.rules.append(&mut contents.rules);
    }

    fn run(&mut self, run: MeasuredRun<T>, shift: f32) {
        self.glyphs.push(PositionedRun {
            origin: [self.width, -run.ascent + shift],
            glyph: run.glyph,
        });
        self.width += run.width;
        self.ascent = self.ascent.max(run.ascent - shift);
        self.descent = self.descent.max(run.descent + shift);
    }

    fn rule(&mut self, start: f32, end: f32, y: f32, thickness: f32) {
        self.rules.push(Rule {
            start: [start, y],
            end: [end, y],
            thickness,
        });
    }
}

type Measure<'a, T> = dyn FnMut(&str, f32, bool, bool) -> MeasuredRun<T> + 'a;

fn measure_run<T>(
    row: &mut Row<T>,
    run: &Run,
    text: &str,
    size: f32,
    measure: &mut Measure<'_, T>,
) {
    row.run(
        measure(text, size * run.scale, run.bold, run.italic),
        -run.shift * size * 0.35,
    );
}

fn node_row<T>(nodes: &[Node], size: f32, measure: &mut Measure<'_, T>, depth: usize) -> Row<T> {
    let mut row = Row::default();
    for node in nodes {
        append_node(&mut row, node, size, measure, depth);
    }
    row
}

fn append_node<T>(
    row: &mut Row<T>,
    node: &Node,
    size: f32,
    measure: &mut Measure<'_, T>,
    depth: usize,
) {
    if depth >= 32 {
        return;
    }
    let x = row.width;
    match node {
        Node::Run(run) => measure_run(row, run, &run.text, size, measure),
        Node::Fraction {
            num, den, scale, ..
        } => {
            // The parser already applies the operand scale.
            let num = node_row(num, size, measure, depth + 1);
            let den = node_row(den, size, measure, depth + 1);
            let em = size * *scale;
            let gap = (em * 0.12).max(1.5);
            let thickness = (em * 0.06).max(1.);
            let mid = -em * 0.35 - scale.shift * size * 0.35;
            let width = num.width.max(den.width) + 4. * *scale;
            let num_y = mid - gap - num.descent;
            let den_y = mid + gap + den.ascent;
            row.ascent = row.ascent.max(num.ascent - num_y);
            row.descent = row.descent.max(den_y + den.descent);
            let num_x = x + (width - num.width) * 0.5;
            let den_x = x + (width - den.width) * 0.5;
            row.append(num, [num_x, num_y]);
            row.append(den, [den_x, den_y]);
            row.rule(x, x + width, mid, thickness);
            row.width += width;
        }
        Node::Sqrt {
            body,
            scale,
            bold,
            italic,
        } => {
            let body = node_row(body, size, measure, depth + 1);
            let em = size * scale;
            let radical = measure("√", em, *bold, *italic);
            let body_x = x + radical.width;
            let right = body_x + body.width;
            let top = -body.ascent - (em * 0.14).max(2.);
            let thickness = (em * 0.06).max(1.);
            row.run(radical, 0.);
            row.ascent = row.ascent.max(-top + thickness * 0.5);
            row.descent = row.descent.max(body.descent);
            row.append(body, [body_x, 0.]);
            row.rule(body_x - 1., right + 1., top, thickness);
            row.width = right + 2.;
        }
    }
}

/// Return glyphs and rules in top-left coordinates. Each explicit row has at
/// least the supplied font line height.
pub fn layout<T>(
    nodes: &[Node],
    size: f32,
    line_height: f32,
    align: TextAlign,
    mut measure: impl FnMut(&str, f32, bool, bool) -> MeasuredRun<T>,
) -> Layout<T> {
    let mut rows = Vec::new();
    let mut row = Row::default();
    for node in nodes {
        if let Node::Run(run) = node {
            for (index, text) in run.text.split('\n').enumerate() {
                if index > 0 {
                    rows.push(std::mem::take(&mut row));
                }
                if !text.is_empty() {
                    measure_run(&mut row, run, text, size, &mut measure);
                }
            }
        } else {
            append_node(&mut row, node, size, &mut measure, 0);
        }
    }
    rows.push(row);

    let width = rows.iter().map(|row| row.width).fold(0., f32::max);
    let mut height = 0.;
    let mut contents = Row::default();
    for row in rows {
        let row_height = (row.ascent + row.descent).max(line_height);
        let x = match align {
            TextAlign::Left => 0.,
            TextAlign::Center => (width - row.width) / 2.,
            TextAlign::Right => width - row.width,
        };
        let baseline = height + (row_height + row.ascent - row.descent) / 2.;
        contents.append(row, [x, baseline]);
        height += row_height;
    }
    Layout {
        size: [width, height],
        glyphs: contents.glyphs,
        rules: contents.rules,
    }
}
