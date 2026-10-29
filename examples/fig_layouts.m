% Positioned axes can be saved as MATLAB FIG (supported lines/scatter).
clf;
t=linspace(0,1,200);
subplot('Position',[0.1 0.62 0.35 0.3]);
plot(t,sin(2*3.141592653589793*3*t),'b-');
title('Input'); xlabel('Time (s)'); ylabel('Amplitude');
subplot('Position',[0.57 0.62 0.35 0.3]);
plot(t,cos(2*3.141592653589793*3*t),'r--');
title('Quadrature'); xlabel('Time (s)'); ylabel('Amplitude');
subplot('Position',[0.1 0.12 0.82 0.33]);
plot(t,sin(2*3.141592653589793*3*t).*exp(-t*3),'k-');
title('Damped signal'); xlabel('Time (s)'); ylabel('Amplitude');
