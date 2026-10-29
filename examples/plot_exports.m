% Relative export paths follow Current Folder (CLI: this script's folder).
f = figure;
t = 0:0.01:2;
plot(t,sin(2*pi*t));
title('Exported signal');
xlabel('Time (s)');
ylabel('Amplitude');
saveas(f,'signal.svg');
saveas(f,'signal.png');
saveas(f,'signal.pdf');
saveas(f,'signal','jpeg');
saveas(f,'signal','tiff');
saveas(f,'signal_plain','tiffn');
saveas(f,'signal.bmp');
saveas(f,'signal.fig');
