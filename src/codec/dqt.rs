use std::array;

#[derive(Debug, PartialEq)]
pub struct SeaDequantTab {
    scale_factor_bits: usize,

    cached_dqt: [Vec<Vec<i32>>; 9],
}

// scale_factors along with residuals should cover all potential values
// we try to calcualte an exponent for max scalefactor that is efficient given the range ot residuals
// theoretically [12, 11, 10, 9, 8, 7] should be fine, but these numbers perform better over a diverse dataset
pub static IDEAL_POW_FACTOR: [f32; 8] = [12.0, 11.65, 11.20, 10.58, 9.64, 8.75, 7.66, 6.63]; // were found experimentally

impl SeaDequantTab {
    pub fn init(scale_factor_bits: usize) -> Self {
        SeaDequantTab {
            scale_factor_bits,
            cached_dqt: array::from_fn(|_| Vec::new()),
        }
    }

    fn calculate_ideal_pow_factors() -> [[f32; 8]; 5] {
        let mut ideal_power_factors: [[f32; 8]; 5] = [[0.0; 8]; 5];

        for scale_factor_bits in 2..=6 {
            for residual_bits in 1..=8 {
                ideal_power_factors[scale_factor_bits - 2][residual_bits - 1] =
                    IDEAL_POW_FACTOR[residual_bits - 1] / (scale_factor_bits as f32)
            }
        }
        ideal_power_factors
    }

    fn calculate_scale_factors(residual_bits: usize, scale_factor_bits: usize) -> Vec<i32> {
        let ideal_pow_factors = Self::calculate_ideal_pow_factors();

        let mut output: Vec<i32> = Vec::new();
        let power_factor = ideal_pow_factors[scale_factor_bits - 2][residual_bits - 1];

        let scale_factor_items = 1 << scale_factor_bits;
        for index in 1..=scale_factor_items {
            let value: f32 = (index as f32).powf(power_factor);
            output.push(value as i32);
        }

        output
    }

    fn get_scalefactor_reciprocals(residual_bits: usize, scale_factor_bits: usize) -> Vec<i32> {
        let scale_factors = Self::calculate_scale_factors(residual_bits, scale_factor_bits);
        let mut output: Vec<i32> = Vec::new();
        for sf in scale_factors {
            let value = ((1 << 16) as f32 / sf as f32) as i32;
            output.push(value);
        }
        output
    }

    fn gen_dqt_table(residual_bits: usize) -> Vec<f32> {
        match residual_bits {
            1 => return vec![2.0],
            2 => return vec![1.115, 4.0],
            _ => (),
        }

        let start: f32 = 0.75f32;
        let steps = 1 << (residual_bits - 1);
        let end = ((1 << residual_bits) - 1) as f32;
        let step = (end - start) / (steps - 1) as f32;
        let step_floor = step.floor();

        let mut curve = vec![0.0; steps];
        for i in 1..steps {
            let y = 0.5 + i as f32 * step_floor;
            curve[i] = y;
        }

        curve[0] = start;
        curve[steps - 1] = end;
        curve
    }

    fn generate_dqt(&self, scale_factor_bits: usize, residual_bits: usize) -> Vec<Vec<i32>> {
        let dqt = Self::gen_dqt_table(residual_bits);

        let scalefactor_items = 1 << scale_factor_bits;

        let mut output: Vec<Vec<i32>> = Vec::new();

        let dqt_items = 2usize.pow(residual_bits as u32 - 1);

        let scale_factors = Self::calculate_scale_factors(residual_bits, scale_factor_bits);

        for s in 0..scalefactor_items {
            output.push(Vec::with_capacity(dqt.len()));

            // zig zag pattern decreases quantization error
            for q in 0..dqt_items {
                let val = (scale_factors[s] as f32 * dqt[q]).round() as i32;
                output[s].push(val);
                output[s].push(-val);
            }
        }

        output
    }

    pub fn get_dqt(&mut self, scale_factor_bits: usize, residual_bits: usize) -> &Vec<Vec<i32>> {
        if scale_factor_bits != self.scale_factor_bits {
            self.cached_dqt = array::from_fn(|_| Vec::new());
        }

        let cached_dqt = &self.cached_dqt[residual_bits as usize];
        if cached_dqt.len() == 0 {
            let new_dqt = self.generate_dqt(scale_factor_bits, residual_bits);
            self.cached_dqt[residual_bits as usize] = new_dqt;
        }

        &self.cached_dqt[residual_bits as usize]
    }
}
