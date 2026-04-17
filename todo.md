- [ ] in processing mode fix images being bigger than the window
- [ ] improve perofmrance for text overlay preview, probably by not doing the full processing pipeline and just doing the text overlay part
- [ ] improve OCR accuracy and performance, expecially with a sticker template

the app seems to work well, look at the code base, at the plan, and improve the little things, and
improve performance, this app will be compiled on the machine that it will run, so it will be
compiled with "-Ctarget-cpu=native", so if you have to write unsafe code or simd code, do so,
performance is critical, also in the numbering part, going to next seems fast, but going prev seems
slow, there should atleast be one prev in cache. Also sometimes the number the OCR needs to find is
quite small and it seems the image that is fed to the OCR is has its quality reduced, but it makes
next to impossible to see the smaller numbers
