pub struct FileHandler {
    mmap: Mmap,
}

impl FileHandler {
    pub fn new(file: &File) -> Result<Self> {
        let mmap = unsafe { Mmap::map(file)? };
        Ok(Self { mmap })
    }

    pub fn read(&self, offset: usize, length: usize) -> &[u8] {
        &self.mmap[offset..offset + length]
    }
}
