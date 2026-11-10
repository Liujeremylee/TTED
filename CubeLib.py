#This is the library for driving the multiplexed display of a common cathode LED Cube using HC595 shift registers
#	the shift register outputs also control a higher current driver for providing power to each layer.
#The columns (and layers) of the cube are mapped to the appropriate bits of the SPI data stream controlling the 595 ICs.
#	this allows the design of the board to be optimized to place the various columns and layers to the nearest 595,
#	rather than having to match the columns in a 1:1 relationship to the outputs of the 595
#	This should allow for using this diver routine with various board designs and sizes of LED "Cubes" or other multiplexed displays.
#
#Code is by Jason Brett (jbrett@bcit.ca) developed for use in BCIT TTED4060 "Teaching Electronics 1" class
#Code is published under theG NU General Public License v3.0
#
#	Commands you can use from your code:
#
#	Required initialization routines:
#		leds=CubeLib.Cube(5)			#creates an initializes a cube object called "leds" (you can call it whatever you want) of size 5x5x5
#										you MUST follow the creation of the cube object with
#		CubeLib.leds = leds				#to pass the leds object back to this routine. (There are neater ways to handle this, but I haven't implemented them.)
#		leds.xy_map=(27,25,24,19,16, 28,30,20,22,23, 29,31,21,17,4, 13,26,3,1,0, 15,14,5,6,7)
#										maps the x (rows) and y (columns) of your cube to the bits in the SPI bitstream that will set them HIGH (off) or LOW (on)
#		leds.layer_map=(9,10,11,8,12)	#maps the layers of your cube to the bits in the SPI bitstream that will set them HIGH (on) or LOW (off)
#											The numbers for xy_map and layer_map are defined by the PCB layout, allowing you to create a board with easy routing
#											and allowing the software to sort out how the HC595 chips are connected to the cube.
#											The values given here are for the Author's 5x5x5 cube design as of Oct. 2026
#	To activate the cube's display you must enter:
#		timer.init(freq=3000, mode=Timer.PERIODIC, callback=CubeLib.refresh_layers) 	#triggers the "refresh_layers" function to run at 3000Hz
#                                        you can set the freq to less than 3000Hz, but setting it higher runs into computational limits on a Pico2
#											If you are running on a PicoH or PicoW (but not a Pico2 or Pico2W or faster) then you will need to lower the refresh rate
#
#	To create your animation you can use:
#		leds.set(x,y,z,6)			#this gives the x(row), y(col) and z (layer) coordinates of the LED you want to set, and the brightness you want to set it to
#										for a 5x5x5 cube running on a Pico2 with 3000Hz refresh rate there are 10 levels of brightness. 0 is off, 10 is "full bright"
#										if you are using a 'regular' Pico with a 1000Hz refresh rate you may need to edit the "if (pwm_value==10):" line (below) to give
#										a lower range of brightness levels. The same goes for cubes larger than 5x5x5
#		leds.dim()					#dims all LEDs in the cube by one brightness level
#		leds.clear()				#sets all LEDs in the cube to "off"		(hmm... that's pretty much the same as leds.fill(0)... maybe this is redundant)
#		leds.fill(6)				#sets all LEDs in the cube to the brightness level in the brackets

from array import array
from machine import Pin, SPI, Timer
import micropython

layer=0					#used to keep track of which layer the refresh_layers function is displaying
pwm_value=1             #used to control the brightness of the leds in the refresh_layers function

        
# Initialize SPI0 (SCK=GP2, MOSI=GP3)
spi = SPI(0, baudrate=20000000, polarity=0, phase=0, sck=Pin(2), mosi=Pin(3))		#these are the settings for the SPI signal that the 
latch = Pin(1, Pin.OUT)																#Pico will use to communicate with the HC595 chips
timer_routine = Pin(10,Pin.OUT)			#This is used to toggle a GPIO output at the beginning and end of the "refresh_layers" routine
                                            #this is used for timing the efficiency of the routine using an oscilloscope or logic analyzer

class Cube:					#This is the "Cube" object class that will be called from your code using something like "leds=CubeLib.Cube(5)"
                                #for a 5x5x5 cube. Remember that you have to "import CubeLib" to import the code from this file to your program
    def __init__(self, size):		#init runs once when the Cube object is first created. This establishes variables that will be used in the object
        self.size = size		#you pass the size of your desired cube when creating the object. This is a place to store that value within the object.
        self.data = array('B', [0] * (size * size * size)) #this creates a one dimensional array of bytes to store the brightness of each LED in the cube
        self.xy_map = array('B', [0] * (size * size))  #this creates an array mapping the cube's columns to the associated bits in the SPI bitstream to the 595s
        self.layer_map = array('B',[0] * size)			#this creates an array mapping the cube's layers to the associated bits in the SPI bitstream to the 595s
        self.bytes_required=(((size*size)+size)//8)+1   #this calculates the number of bytes required to store the SPI bitstream.
                                                            #so 5 rows x 5 columns + 5 layers = 30 bits required. // is Integer division, giving 3. Adding one
                                                            #gives four bytes, or 32 bits... enough to store and send the 30 bits required for your cube
        self.buf = bytearray(self.bytes_required)			#this creates the buffer that will store the bitstream so it can be quickly sent using the SPI routine

    @micropython.native		#Micropython's "native" mode runs faster than the regular intrepreted mode. Speed improvements matter for smooth multiplexed displays.
    def _index(self, x, y, z):	#the _index function is called by routines below. It maps a 3 dimensional coordinate to the one dimensional array storing the brightness of each LED
        if not (0 <= x < self.size and 0 <= y < self.size and 0 <= z < self.size):	#this checks to make sure that the LED being referred to actually exists... 4,3,8 does not work for a 5x5x5 cube
            raise IndexError("Coordinates out of bounds")
        return z * self.size * self.size + y * self.size + x	#this returns the location of the byte storing that LED's brightness within the array
    
    @micropython.native
    def set(self, x, y, z, value):		#
        self.data[self._index(x, y, z)] = value
    @micropython.viper
    def get(self, x, y, z):
        return self.data[self._index(x, y, z)]

    def clear(self):
        for i in range(len(self.data)):
            self.data[i] = 0
            
    def dim(self,step=1):
        for i in range(len(self.data)):
            if (self.data[i] >= step):
                self.data[i]=self.data[i]-step
            else:
                self.data[i]=0
    
    def fill(self, value=1):
        for i in range(len(self.data)):
            self.data[i] = value

    def layer(self, z):
        """Returns a 2D list of a single Z-layer"""
        return [[self.get(x, y, z) for x in range(self.size)] for y in range(self.size)]

    def __str__(self):
        out = ""
        for z in range(self.size):
            out += f"Layer {z}:\n"
            for y in range(self.size):
                row = [str(self.get(x, y, z)) for x in range(self.size)]
                out += " ".join(row) + "\n"
            out += "\n"
        return out
        

  
    @micropython.viper
    def display_layer(self, layer:int, pwm_value:int):
        bits:int = 0									#this sets bits, the data to be sent to the cube, so all bits are zero. The layers will be low (off), but the columns and rows will also be low (on) 
        bits |= 1 << int(self.layer_map[layer])         #this turns on the active layer for this iteration of the routine

        size:int = int(self.size)        # cast to native int
        base:int = layer * size * size

        for i in range(size * size):     # now valid
#             if (int(self.data[base + i]) == 0):			#checks if the value for the bit is zero
            if (int(self.data[base + i]) <= pwm_value):
                bits |= 1 << int(self.xy_map[i])						#sets the bit for the LED's row/column value to one, shifting it high so it no longer sinks current.. LED is OFF

        #buf = bytearray(4)
        buf=self.buf
        buf[0] = (bits >> 24) & 0xFF
        buf[1] = (bits >> 16) & 0xFF
        buf[2] = (bits >> 8) & 0xFF
        buf[3] = bits & 0xFF

        latch.value(0)
        spi.write(buf)
        latch.value(1)

@micropython.native
def refresh_layers(t):      #called by timer interrupt, takes about 500uS to operate so only call at 1000Hz max
    timer_routine.value(1)
    global layer
    global pwm_value
    leds.display_layer(layer,pwm_value)
    layer=layer+1
    if (layer==5):
        layer=0
        pwm_value=pwm_value+1
        if (pwm_value==10):
            pwm_value=0
    timer_routine.value(0)
    
        


