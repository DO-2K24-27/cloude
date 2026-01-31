use std::u32;
use std::env;

use vmm::VMM;

#[derive(Debug)]
pub enum Error {
    VmmNew(vmm::Error),

    VmmKernel(env::VarError),
    
    VmmConfigure(vmm::Error),

    VmmRun(vmm::Error),
}

fn main() -> Result<(), Error> {

    let mut vmm = VMM::new().map_err(Error::VmmNew)?;

    let kernel_path = env::var("KERNEL_PATH").map_err(Error::VmmKernel)?;

    vmm.configure(4, 512, &kernel_path)
        .map_err(Error::VmmConfigure)?;

    vmm.run();

    Ok(())
}
