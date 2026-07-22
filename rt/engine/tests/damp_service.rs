use open_conv_engine::*;

fn run(damp: f64) -> Vec<f32> {
    let sr = 48000.0;
    let mut e = Engine::new_sized(sr, 1, 64, 0.1);
    let mut s = 7u64;
    let ir: Vec<f32> = (0..2048)
        .map(|i| {
            s ^= s >> 12; s ^= s << 25; s ^= s >> 27;
            let v = (s.wrapping_mul(0x2545F4914F6CDD1D) >> 40) as f64 / (1u64 << 24) as f64;
            ((v * 2.0 - 1.0) * (-(i as f64) / 600.0).exp()) as f32
        })
        .collect();
    e.set_source_ir(0, vec![ir], sr, 1.0);
    let p = EngineParams { n_zones: 1, wet: 1.0, dry: 0.0, damp: [damp; MAX_ZONES], ..Default::default() };
    let mut s2 = 42u64;
    let x: Vec<f32> = (0..48000)
        .map(|_| {
            s2 ^= s2 >> 12; s2 ^= s2 << 25; s2 ^= s2 >> 27;
            ((s2.wrapping_mul(0x2545F4914F6CDD1D) >> 40) as f64 / (1u64 << 24) as f64 - 0.5) as f32 * 0.5
        })
        .collect();
    let mut out = Vec::new();
    for chunk in x.chunks(64) {
        e.service(&p);
        let mut buf = chunk.to_vec();
        let mut io = [buf.as_mut_slice()];
        e.process_block(&mut io, &p);
        out.extend_from_slice(&buf);
    }
    out
}

#[test]
fn service_applies_damp() {
    let y0 = run(0.0);
    let y1 = run(1.0);
    let tail0: f64 = y0[24000..].iter().map(|v| (*v as f64).powi(2)).sum();
    let tail1: f64 = y1[24000..].iter().map(|v| (*v as f64).powi(2)).sum();
    let diff: f64 = y0[24000..].iter().zip(&y1[24000..]).map(|(a, b)| ((a - b) as f64).powi(2)).sum();
    assert!(
        diff > 0.01 * tail0.max(tail1),
        "damp had no effect: tail0={tail0} tail1={tail1} diff={diff}"
    );
}
